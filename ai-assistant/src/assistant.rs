//! AI助手核心实现
//! 状态机自驱动模式：业务逻辑在各状态的 `on_enter` / `on_transition` 中实现。
//! `process()` 只负责：推入用户消息 → tick 循环 → 返回响应。
//! 流程：
//!   asking (等待用户) → thinking (调 LLM) → executing / asking / result
//!   executing → thinking → ... → result → asking（下一轮）

use crate::context::{keys, AssistantContext};
use crate::skills::{init_skill_manager, SkillManager};
use crate::state::{events, states};
use crate::state_machine;
use crate::{AIAssistantConfig, Result};

use corework::cache::{Cache, CacheExt};
use corework::error::FrameworkError;
use corework::execution_unit::ExecutionUnit;
use corework::statemachine::StateMachine;
use std::sync::Arc;

/// 默认最大 thinking 循环次数（0 = 无限制）
const DEFAULT_MAX_THINKING_ROUNDS: u32 = 0;

/// 默认最大历史消息条数
const DEFAULT_MAX_HISTORY_MESSAGES: usize = 200;

/// AI助手（管理层）
/// 职责划分：
/// - **管理层**：SkillManager 初始化、持久化（会话管理/恢复/归档）、全局资源
/// - **Boss Agent**：状态机运行时（cache 里的 MODEL/MAIN_SKILLS/ACTIVE_TOOLS 等）
/// `process()` 驱动 Boss 状态机 tick 循环，`restore_session()` 负责持久化恢复。
pub struct AIAssistant {
    /// 配置
    config: AIAssistantConfig,
    /// Boss 状态机（独立执行单元，拥有自己的 ScopedCache）
    state_machine: Option<Arc<StateMachine>>,
    /// main 层 skill 名称（activate_skills 时保存，reset 时自动重新激活）
    main_skill_names: Vec<String>,
    /// Parent conversation execution unit.
    parent_unit: Option<Arc<ExecutionUnit>>,
}

impl AIAssistant {
    /// 创建新的 AI 助手实例
    pub fn new(config: AIAssistantConfig) -> Self {
        Self {
            config,
            state_machine: None,
            main_skill_names: Vec::new(),
            parent_unit: None,
        }
    }

    /// 使用默认配置创建
    pub fn with_defaults() -> Self {
        Self::new(AIAssistantConfig::default())
    }

    pub fn with_parent_unit(mut self, parent_unit: Arc<ExecutionUnit>) -> Self {
        self.parent_unit = Some(parent_unit);
        self
    }

    /// 初始化/重建 Boss 状态机
    /// 首次调用时还会初始化全局 SkillManager（OnceLock 保证只执行一次）。
    /// 若 config.data_dir 已设置，同步注入到持久化层。
    /// **不涉及持久化**——会话恢复由 `restore_session()` 单独负责。
    pub async fn init(&mut self) -> Result<()> {
        // ---- 将 data_dir 注入持久化层（OnceLock，只有首次生效） ----
        if let Some(ref dir) = self.config.data_dir {
            crate::persistence::set_data_dir(dir.clone());
        }
        if let Some(ref dir) = self.config.prompts_dir {
            crate::prompt_assets::set_prompts_dir(dir.clone());
        }
        let language = crate::prompt_assets::set_language(&self.config.language)
            .map_err(|e| FrameworkError::InvalidOperation(e.into()))?;

        // ---- 初始化 SkillManager（仅首次生效，OnceLock 保护） ----
        self.ensure_skill_manager().await?;

        let mut builder = state_machine::build_assistant_state_machine();
        if let Some(parent_unit) = &self.parent_unit {
            builder = builder.with_parent_unit(parent_unit.clone());
        }
        let sm = Arc::new(builder.build().await?);
        sm.start().await?; // 进入初始状态 suspended

        // 写入 Boss Agent 运行时配置到 cache
        let cache = sm.unit().cache();
        cache
            .set(
                keys::MAX_HISTORY_MESSAGES,
                &DEFAULT_MAX_HISTORY_MESSAGES,
                None,
            )
            .await?;
        cache
            .set(
                keys::MAX_THINKING_ROUNDS,
                &DEFAULT_MAX_THINKING_ROUNDS,
                None,
            )
            .await?;
        cache.set(keys::THINKING_ROUND_COUNT, &0u32, None).await?;
        cache
            .set(keys::RETRIEVAL_CONFIG, &self.config.retrieval, None)
            .await?;
        cache
            .set(
                keys::FRONTEND_WIDGETS_ENABLED,
                &self.config.effective_frontend_widgets_enabled(),
                None,
            )
            .await?;
        cache
            .set(keys::SYSTEM_SKILLS, &self.config.system_skills, None)
            .await?;
        // Initialize host-owned dynamic text fields for this agent.
        cache
            .set(
                keys::HOST_DYNAMIC_SNAPSHOTS,
                &std::collections::HashMap::<String, String>::new(),
                None,
            )
            .await?;
        // feat/line-protocol：JSON prompt 模式默认走行式协议
        // FC 模式（支持 tool_choice 的模型）不受此键影响
        cache
            .set(keys::DECISION_PROTOCOL, &"line".to_string(), None)
            .await?;
        cache.set(keys::LANGUAGE, &language, None).await?;
        cache
            .set(
                keys::PROMPTS_DIR,
                &crate::prompt_assets::prompts_dir().display().to_string(),
                None,
            )
            .await?;
        if let Some(data_dir) = self.config.data_dir.as_ref() {
            cache
                .set(
                    keys::RUNTIME_DATA_DIR,
                    &data_dir.display().to_string(),
                    None,
                )
                .await?;
        }

        self.state_machine = Some(sm);
        Ok(())
    }

    /// 恢复持久化会话（管理层职责）
    /// 从磁盘加载上次会话或创建新会话，将 Boss 对话历史注入到状态机 cache。
    /// 返回 `RestoreResult`，其中 `active_agents` 供调用方决定是否重建子 Agent。
    /// 必须在 `init()` + `activate_skills()` 之后调用。
    pub async fn restore_session(&self) -> Result<crate::persistence::RestoreResult> {
        let sm = self
            .state_machine
            .as_ref()
            .ok_or_else(|| FrameworkError::InvalidOperation("状态机未初始化".into()))?;
        let cache = sm.unit().cache();

        match crate::persistence::init_persistence(&self.config.model, &self.config.agent_id).await
        {
            Ok(mut restore) => {
                // 1) 优先用 cache snapshot 一次性恢复默认 Agent 全部 runtime 字段
                //    （language / host_dynamic_snapshots / model / imported_skills /
                //     active_tools / current_plan / 焦点 / conversation）；
                //    snapshot 内部已做归一化和瞬态字段裁剪。
                let session_id = crate::persistence::current_session_id();
                let snapshot_restored = if !session_id.is_empty() {
                    crate::persistence::restore_cache_snapshot(
                        &session_id,
                        &self.config.agent_id,
                        &cache,
                    )
                    .await
                    .unwrap_or(false)
                } else {
                    false
                };

                // 2) 用 ledger 校准 conversation（来自 jsonl 的中心 ledger 是规范源），
                //    防止 snapshot 时机滞后于最新一条 ledger。归一化裁剪 dangling tool。
                let trimmed_history = crate::persistence::normalize_messages_for_recovery(
                    std::mem::take(&mut restore.default_agent_history),
                );
                let entry = crate::persistence::recovery_entry_from_messages(&trimmed_history);

                if !trimmed_history.is_empty() {
                    tracing::debug!(
                        "恢复默认 Agent 会话：snapshot={} ledger={}条（截断到 {:?} 入口）",
                        snapshot_restored,
                        trimmed_history.len(),
                        entry,
                    );
                    AssistantContext::set_conversation(&cache, &trimmed_history).await?;
                } else if snapshot_restored {
                    // ledger 空但 snapshot 有 conversation：以 snapshot 为准（已归一化）。
                    tracing::debug!("恢复默认 Agent 会话：仅 snapshot（ledger 为空）");
                }
                restore.default_agent_history = trimmed_history;

                // 3) 旧 pod 残留的 pending tool / next_state 等瞬态字段必须清掉。
                //    snapshot 路径已自动清理，这里兜底覆盖 ledger-only 的恢复路径。
                crate::persistence::clear_recovery_transient_keys(&cache).await?;

                // 4) 没有 snapshot 时，imported_skills / current_plan 才需要从
                //    SessionMeta 重放；有 snapshot 时这些字段已经在 cache 里。
                if !snapshot_restored {
                    if !restore.imported_skills.is_empty() {
                        let skills_arg = restore.imported_skills.join(",");
                        let cmd = format!("UpdateSkills --skills {}", skills_arg);
                        tracing::debug!(
                            skill_count = restore.imported_skills.len(),
                            "restore imported skills"
                        );
                        let ctx = sm.unit().create_context();
                        let result = crate::tool_runner::execute_single(&cmd, &ctx).await;
                        if !result.success {
                            tracing::warn!("恢复 imported skills 失败: {}", result.to_ai);
                        }
                    }

                    if let Some(plan) = &restore.current_plan {
                        AssistantContext::set_current_plan(&cache, plan).await?;
                    }
                }

                // 5) 末尾是 user：SM 已在 init() 中以 SUSPENDED 启动，
                //    这里发一个 USER_INPUT 事件把它推到 thinking，由调用方 tick 驱动 LLM。
                if matches!(entry, crate::persistence::RecoveryEntry::Thinking)
                    && sm.current_state() == states::SUSPENDED
                {
                    if let Err(e) = sm.send_event(events::USER_INPUT).await {
                        tracing::warn!("恢复时触发 thinking 失败: {}", e);
                    }
                }

                Ok(restore)
            }
            Err(e) => {
                tracing::warn!("持久化层初始化失败，使用空会话: {}", e);
                Ok(crate::persistence::RestoreResult {
                    default_agent_history: Vec::new(),
                    active_agents: Vec::new(),
                    imported_skills: Vec::new(),
                    current_plan: None,
                })
            }
        }
    }

    /// 重置对话（归档当前会话 + 重建状态机 + 创建新会话）
    /// 内部自动重新激活 main skills，调用方无需再次调用 `activate_skills()`。
    pub async fn reset_conversation(&mut self) -> Result<()> {
        // 1. 归档当前会话
        if let Some(cache) = self.cache() {
            // 归档前先把默认 Agent 的 cache 快照刷盘，保证 language 等持久状态
            // 等运行时字段不丢，恢复路径与子 Agent 一致。
            self.persist_cache_snapshot().await;
            let history = AssistantContext::get_conversation(&cache).await?;
            let title = crate::persistence::auto_title(&history);
            if let Err(e) = crate::persistence::archive_current_session(&title).await {
                tracing::warn!("归档会话失败: {}", e);
            }
        }

        // 2. 重建状态机（全新 cache）
        self.init().await?;

        // 3. 重新激活 main skills（init 创建了全新 cache，需要重新写入）
        if !self.main_skill_names.is_empty() {
            let names: Vec<String> = self.main_skill_names.clone();
            let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            self.activate_skills(&refs).await?;
        }

        // 4. 创建新会话
        crate::persistence::create_new_session(&self.config.model).await?;

        Ok(())
    }

    /// 激活 main 层 Skills 及其关联工具
    /// 由调用方（如 Tauri 层）在 `init()` 之后调用一次，传入应用层定义的 main skills。
    /// 写入 `MAIN_SKILLS`（始终存在，不受 UpdateSkills 影响）+ `ACTIVE_TOOLS`。
    /// 同时保存到 `self.main_skill_names`，`reset_conversation()` 时自动重新激活。
    pub async fn activate_skills(&mut self, skill_names: &[&str]) -> Result<()> {
        // 保存一份，reset 时自动重新激活
        self.main_skill_names = skill_names.iter().map(|s| s.to_string()).collect();

        let cache = self
            .cache()
            .ok_or_else(|| FrameworkError::InvalidOperation("状态机未初始化".into()))?;
        self.activate_skills_with_cache(skill_names, &cache).await
    }

    /// 内部实现（接受 cache 引用，避免重复获取）
    async fn activate_skills_with_cache(
        &self,
        skill_names: &[&str],
        cache: &Arc<dyn Cache>,
    ) -> Result<()> {
        use crate::skills::systems::SKILL_MANAGER;

        let mgr_lock = match SKILL_MANAGER.get() {
            Some(m) => m,
            None => return Ok(()),
        };

        let mut mgr = mgr_lock.write().await;

        if skill_names.is_empty() {
            tracing::debug!("activate_skills: 列表为空，跳过");
            return Ok(());
        }

        if let Err(e) = mgr.load_many(skill_names).await {
            tracing::warn!("activate_skills 加载部分 Skills 失败: {}", e);
        }

        let mut tools = Vec::new();
        mgr.inject_tools_for_skills(skill_names, &mut tools);
        mgr.inject_tools_for_state(crate::state::states::THINKING, &mut tools);
        tools.sort();
        tools.dedup();

        tracing::debug!(
            skill_count = skill_names.len(),
            tool_count = tools.len(),
            "main skills activated"
        );

        // 写入 MAIN_SKILLS（独立于 IMPORTED_SKILLS，不被 UpdateSkills 覆盖）
        let names_owned: Vec<String> = skill_names.iter().map(|s| s.to_string()).collect();
        cache.set(keys::MAIN_SKILLS, &names_owned, None).await?;
        AssistantContext::set_active_tools(cache, tools).await?;

        Ok(())
    }

    /// 确保全局 SkillManager 已初始化
    /// Skills 目录解析优先级（和 browser-automation runtime 保持一致）：
    /// 1. 环境变量 `SUNWOO_SKILLS_DIR` — 开发期覆盖或自定义部署
    /// 2. `<exe目录>/resources/skills/` — Tauri 打包后 resources 目录
    /// 3. `%APPDATA%\sunwoo\skills\` — 用户数据目录（动态扩展，打包内无此目录则用这里）
    /// 4. `config.skills_dir` — 回退到配置值（开发期相对路径）
    /// 优先级 3 的目录用户可随时增删 skill，无需重新打包发布应用。
    async fn ensure_skill_manager(&self) -> Result<()> {
        // OnceLock::get 快路径：已初始化直接返回
        use crate::skills::systems::SKILL_MANAGER;
        if SKILL_MANAGER.get().is_some() {
            return Ok(());
        }

        let skills_dir =
            resolve_skills_dir(&self.config.skills_dir, self.config.data_dir.as_deref());
        let skills_dir = &skills_dir;
        let registry_path = skills_dir.join("skills.json");

        let manager = if registry_path.exists() {
            tracing::debug!(path = %registry_path.display(), "load skills from registry");
            SkillManager::from_registry(&registry_path).await?
        } else {
            tracing::debug!(path = %skills_dir.display(), "load skills from directory");
            SkillManager::from_directory(skills_dir).await?
        };

        let discovered_count = manager.len();
        init_skill_manager(manager);
        tracing::debug!(discovered_count, "skill manager initialized");

        Ok(())
    }

    /// 获取配置
    pub fn config(&self) -> &AIAssistantConfig {
        &self.config
    }

    /// 切换当前会话使用的推理模型。
    /// 写入路径优先级（见 docs/AGENT_GATEWAY_ADMISSION.md §5）：
    /// 1. 当前 `Conversation::global()` 存在 → 写入 conversation 层
    ///    `config:model`，作用域仅本会话；
    /// 2. 同步写入 default agent 自身 cache 的 `keys::MODEL`，便于持久化。
    /// 不再写入全局 `llm_gateway::key_store::set_current`，避免在
    /// multi-conversation 场景下"一会话切换 → 其它会话被静默传染"。
    pub async fn set_model(&self, model: &str) -> Result<()> {
        // 校验模型名是否在 key_store 注册（保留报错语义）。
        llm_gateway::key_store::find_by_name(model).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("未找到模型 '{}' 的 uid", model).into())
        })?;
        crate::config_resolver::write_conversation_model(model).await?;
        if let Some(cache) = self.cache() {
            cache.set(keys::MODEL, &model.to_string(), None).await?;
        }
        Ok(())
    }

    /// 获取当前会话使用的推理模型名（按 §5 三层 fallback 解析）。
    pub async fn get_model(&self) -> Result<String> {
        if let Some(cache) = self.cache() {
            if let Some(uid) = crate::config_resolver::resolve_inference_model_uid(&cache).await {
                if let Some(entry) = llm_gateway::key_store::get(uid) {
                    return Ok(entry.model_name);
                }
            }
        }
        Ok(String::new())
    }

    pub async fn set_language(&self, language: &str) -> Result<String> {
        let normalized = crate::prompt_assets::set_language(language)
            .map_err(|e| FrameworkError::InvalidOperation(e.into()))?;
        // 写入 conversation 层（多 agent 共享）。
        crate::config_resolver::write_conversation_language(&normalized).await?;
        // 同步写 default agent 自身 cache，兼容已有读取路径与持久化。
        if let Some(cache) = self.cache() {
            cache.set(keys::LANGUAGE, &normalized, None).await?;
        }
        Ok(normalized)
    }

    /// 获取状态机的 ScopedCache
    pub fn cache(&self) -> Option<Arc<dyn Cache>> {
        self.state_machine.as_ref().map(|sm| sm.unit().cache())
    }

    /// 获取状态机执行单元 ID
    pub fn unit_id(&self) -> Option<String> {
        self.state_machine
            .as_ref()
            .map(|sm| sm.unit().id().to_string())
    }

    /// 获取当前状态名
    pub fn current_state(&self) -> Option<String> {
        self.state_machine.as_ref().map(|sm| sm.current_state())
    }

    pub fn state_machine(&self) -> Option<Arc<StateMachine>> {
        self.state_machine.as_ref().map(Arc::clone)
    }

    /// 把默认 Agent 的整张 ScopedCache 持久化到
    /// `<conversations>/snapshots/{sid}/{agent_id}.json`。
    /// 与子 Agent 的 `AgentRuntime::persist_cache_snapshot` 完全对称，
    /// 让 conversation 焦点、language、imported_skills/active_tools、
    /// model 切换、current_plan 等所有运行时字段都进盘，恢复时单一来源。
    pub async fn persist_cache_snapshot(&self) {
        let session_id = crate::persistence::current_session_id();
        if session_id.is_empty() {
            return;
        }
        let Some(cache) = self.cache() else {
            return;
        };
        if let Err(e) =
            crate::persistence::save_cache_snapshot(&session_id, &self.config.agent_id, &cache)
                .await
        {
            tracing::warn!(
                "save default agent cache snapshot failed for {}: {}",
                self.config.agent_id,
                e
            );
        }
    }

    pub async fn request_pause(&self) -> Result<()> {
        let sm = self
            .state_machine
            .as_ref()
            .ok_or_else(|| FrameworkError::InvalidOperation("状态机未初始化".into()))?;
        let cache = sm.unit().cache();
        let state = sm.current_state();
        let task_status: Option<String> = cache.get(keys::TASK_STATUS).await?;
        if state == states::THINKING
            || state == states::EXECUTING
            || task_status.as_deref() == Some("running")
        {
            crate::state::request_pause(&cache, None).await?;
        } else {
            crate::state::request_pause(&cache, Some(sm)).await?;
        }
        // 暂停后立即刷盘：避免 pod 在 pause 与下一次 process 之间挂掉时，
        // 默认 Agent 的 language / current_plan 等持久字段丢失。
        self.persist_cache_snapshot().await;
        Ok(())
    }

    /// 处理用户输入 — 自驱动循环
    /// 1. 设置 WAITING_FOR_INPUT = false
    /// 2. 用户消息推入对话历史
    /// 3. `send_event(USER_INPUT)` → 进入 thinking（on_enter 调 LLM + 写决策）
    /// 4. `tick()` 循环：on_transition 自动转移 → 新状态 on_enter 执行
    /// 5. 状态机停在 asking（on_enter 设置 WAITING_FOR_INPUT = true）
    /// 6. 读取 PENDING_RESPONSE 返回
    pub async fn process(&mut self, input: &str) -> Result<String> {
        let sm = self
            .state_machine
            .as_ref()
            .ok_or_else(|| FrameworkError::InvalidOperation("状态机未初始化".into()))?;
        let cache = sm.unit().cache();

        // 1. 收到用户输入，关闭等待标记
        cache.set(keys::WAITING_FOR_INPUT, &false, None).await?;
        cache
            .set(keys::TASK_STATUS, &"running".to_string(), None)
            .await?;
        cache.set(keys::AUTO_CONTINUE_STEPS, &0u32, None).await?;
        cache.delete(keys::LAST_STOP_REASON).await?;
        cache.delete(keys::NEXT_STATE_AFTER_SAYING).await?;

        // 2. 用户回答始终以 role:user 推入。
        // asking 状态的 on_enter 已将 assistant_decide 对应的 tool 配对消息
        // （"正在向用户询问问题: xxx"）推入对话历史并清除了 PENDING_TOOL_CALL_ID。
        // 空字符串跳过推入（用于 AgentReport 已手动注入消息后触发 thinking 的场景）
        if !input.is_empty() {
            let event_bus = sm.unit().event_bus();
            AssistantContext::push_user_message_on_event_bus(&cache, &event_bus, input).await?;
        }

        if sm.current_state() == states::SAYING {
            sm.tick().await?;
        }

        // 3. asking → thinking（触发 thinking 的 on_enter）
        if sm.current_state() == states::SUSPENDED || sm.current_state() == states::SAYING {
            sm.send_event(events::USER_INPUT).await?;
        }

        // 4. 自驱动循环：tick() 触发 on_transition → 自动转移 → on_enter
        //    注意：result 有 on_transition → asking，所以 result 也会自动转到 asking
        //    最终停在 asking（asking 无 on_transition）
        loop {
            let current = sm.current_state();
            // 稳态退出：
            // - asking：AI 自然停下（thinking 未调工具 / result 收尾）
            // - suspended：用户主动暂停 / 子 Agent 报告等待
            if current == states::SAYING || current == states::SUSPENDED {
                break;
            }
            if let Err(e) = sm.tick().await {
                tracing::error!(
                    "状态机 tick 出错 (当前状态={}): {}, 强制恢复到 asking",
                    sm.current_state(),
                    e
                );
                // 错误恢复：强制回到 asking，避免状态机卡死无法接受下次 user_input
                if let Err(recover_err) = sm.force_state(states::SUSPENDED).await {
                    tracing::error!("force_state 恢复失败: {}", recover_err);
                    return Err(e.into());
                }
                cache.set(keys::WAITING_FOR_INPUT, &true, None).await?;
                let error_text = e.to_string();
                cache
                    .set(
                        keys::PENDING_RESPONSE,
                        &crate::prompt_assets::render(
                            "runtime_error_response.md",
                            &[("{{ERROR}}", &error_text)],
                        ),
                        None,
                    )
                    .await?;
                break;
            }
        }

        // 5. 返回待展示文本
        let response: String = cache.get(keys::PENDING_RESPONSE).await?.unwrap_or_else(|| {
            crate::prompt_assets::template("empty_runtime_response.md")
                .trim()
                .to_string()
        });

        // 6. 一轮收尾刷盘：默认 Agent 与子 Agent 对称落盘，
        //    保证下次冷启动能用 snapshot 一次性恢复全部 runtime 字段。
        self.persist_cache_snapshot().await;

        Ok(response)
    }

    /// 兼容旧复命入口。
    /// 新模型中复命由 `ReportToAgent` 直接写入目标 Agent 历史并切换焦点；
    pub async fn process_agent_report(&mut self) -> Result<String> {
        self.process("").await
    }

    /// 流式模式处理用户输入（P3-1）
    /// 与 `process` 相同，但在 thinking 状态调用 LLM 时会流式推送 content delta。
    /// 每个 delta 通过 `on_chunk` 回调返回，可直接用于 `app.emit`。
    /// ## 使用方式
    /// ```ignore
    /// assistant.process_streaming("你好", |chunk| {
    ///     app.emit("ai:chunk", chunk).ok();
    /// }).await?;
    /// ```
    pub async fn process_streaming<F>(&mut self, input: &str, mut on_chunk: F) -> Result<String>
    where
        F: FnMut(String) + Send + 'static,
    {
        use tokio::sync::mpsc;

        // 建立 chunk 通道，容量 256（写满时 try_send 丢弃，不阻塞 LLM）
        let (tx, mut rx) = mpsc::channel::<String>(256);

        // 注册 sender 到 thinking 的全局静态（on_chunk 在 LLM streaming 回调中调用）
        crate::state::thinking::set_stream_sender(Some(tx));

        // spawn 一个转发 task：从 rx 读 chunk，调用 on_chunk
        let forward = tokio::spawn(async move {
            while let Some(chunk) = rx.recv().await {
                on_chunk(chunk);
            }
        });

        // 运行正常 process 逻辑（状态机自驱动）
        let result = self.process(input).await;

        // 清除 sender（停止流式）
        crate::state::thinking::set_stream_sender(None);

        // 等待 forward task 把所有 chunk 消费完
        let _ = forward.await;

        result
    }
}

// ============================================================================
// Skills 目录解析
// ============================================================================

/// 解析运行时 Skills 目录，三段式优先级：
/// 1. 环境变量 `SUNWOO_SKILLS_DIR` — 完全覆盖，指定单一目录
/// 2. `<data_dir>/skills/` — 由外部（Tauri）传入的 app_local_data_dir，首次运行从打包资源同步
/// 3. `fallback`（config.skills_dir，开发期相对路径）
fn resolve_skills_dir(
    fallback: &std::path::Path,
    data_dir: Option<&std::path::Path>,
) -> std::path::PathBuf {
    // 1) 开发期覆盖
    if let Ok(custom) = std::env::var("SUNWOO_SKILLS_DIR") {
        let p = std::path::PathBuf::from(custom);
        if p.exists() {
            tracing::info!("Skills 目录(env): {}", p.display());
            return p;
        }
    }

    // 2) 外部注入的 data_dir（Tauri app_local_data_dir）
    if let Some(base) = data_dir {
        let user_skills = base.join("skills");
        // 找到打包资源目录，同步尚未存在的 skill 文件
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                let mut packaged = exe_dir.join("resources").join("skills");
                if !packaged.exists() {
                    let mut cur = exe_dir.to_path_buf();
                    for _ in 0..8 {
                        let probe = cur.join("skills");
                        if probe.exists() {
                            packaged = probe;
                            break;
                        }
                        if !cur.pop() {
                            break;
                        }
                    }
                }
                if packaged.exists() {
                    if let Err(e) = sync_skills_from_packaged(&packaged, &user_skills) {
                        tracing::warn!("Skills 同步失败: {}", e);
                    }
                }
            }
        }
        if user_skills.exists() {
            tracing::info!("Skills 目录(data_dir): {}", user_skills.display());
            return user_skills;
        }
    }

    // 3) 回退：config.skills_dir（开发期相对路径）
    tracing::info!("Skills 目录(fallback): {}", fallback.display());
    fallback.to_path_buf()
}

/// 将打包资源中的 skills 同步到用户可写目录。
/// 仅复制目标目录中不存在的文件/目录，已存在的不覆盖（保留用户修改）。
fn sync_skills_from_packaged(
    src: &std::path::Path,
    dst: &std::path::Path,
) -> std::result::Result<(), String> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)
            .map_err(|e| format!("创建 Skills 目录失败 {}: {}", dst.display(), e))?;
    }
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if dst_path.exists() {
            // 已存在则跳过，保留用户修改
            continue;
        }
        if src_path.is_dir() {
            sync_skills_from_packaged(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("复制 {} 失败: {}", src_path.display(), e))?;
        }
    }
    Ok(())
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_assistant_creation() {
        let assistant = AIAssistant::with_defaults();
        assert_eq!(assistant.config().name, "AI助手");
    }

    #[tokio::test]
    async fn test_assistant_init() {
        let mut assistant = AIAssistant::with_defaults();
        let result = assistant.init().await;
        assert!(result.is_ok());
        // 当前初始等待态应为 suspended
        assert_eq!(assistant.current_state(), Some("suspended".to_string()));
    }

    #[tokio::test]
    async fn test_rebuild_resets_context() {
        let mut assistant = AIAssistant::with_defaults();
        assistant.init().await.unwrap();
        let id1 = assistant.unit_id().unwrap().to_string();

        // 重建 = 新对话，新 ID
        assistant.init().await.unwrap();
        let id2 = assistant.unit_id().unwrap().to_string();
        assert_ne!(id1, id2, "重建后应产生新的执行单元 ID");
    }
}
