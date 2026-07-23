//! 对话历史持久化模块
//! 负责会话 Ledger、默认 Agent 状态和子 Agent 快照的磁盘读写。
//! ## 目录结构
//! ```text
//! %LOCALAPPDATA%/Lumi/conversations/
//! ├── index.json                         ← 会话索引（所有会话元数据）
//! ├── sessions/
//! │   └── {session_id}.jsonl             ← 单会话 Ledger，按 agent_id 区分来源
//! └── snapshots/
//!     └── {session_id}/
//!         └── {agent_id}.json            ← Agent cache 快照
//! ```
//! ## 设计原则
//! - **JSONL append-write**：写入只追加一行，O(1)，崩溃安全
//! - **异步不阻塞**：`push_message` 里 `tokio::spawn` 追加，不等结果
//! - **子 Agent 按状态恢复**：Completed/Failed 只展示，Interactive + Running/Idle 重建
//! - **当前计划持久化**：会话元数据中保存当前计划，恢复时一起恢复

use crate::agent::{AgentClass, AgentPermissions, AgentStatus};
use crate::context::{CurrentPlan, Message};
use crate::ledger::LedgerRecord;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

static AUTO_FILE_PERSISTENCE_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_auto_file_persistence_enabled(enabled: bool) {
    AUTO_FILE_PERSISTENCE_ENABLED.store(enabled, Ordering::SeqCst);
}

pub fn auto_file_persistence_enabled() -> bool {
    AUTO_FILE_PERSISTENCE_ENABLED.load(Ordering::SeqCst)
}

// ============================================================================
// 数据结构
// ============================================================================

/// 会话索引（持久化到 index.json）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationIndex {
    /// 格式版本
    pub version: u32,
    /// 当前活跃的会话 ID
    pub current_session: Option<String>,
    /// 所有会话的元数据
    pub sessions: Vec<SessionMeta>,
}

#[cfg(test)]
mod legacy_migration_tests {
    use super::*;

    #[test]
    fn legacy_persisted_message_migrates_to_ledger() {
        let pm = PersistedMessage {
            ledger: None,
            inner: Message::user("legacy"),
            agent_id: Some("agent_1".to_string()),
            display: None,
        };

        let ledger = pm.into_ledger(7);
        assert_eq!(ledger.record_id, 7);
        assert_eq!(ledger.agent_id, "agent_1");
        assert_eq!(ledger.role, crate::ledger::LedgerRole::User);
        assert_eq!(ledger.content, "legacy");
    }

    #[tokio::test]
    async fn append_and_load_full_session_roundtrips_ledger() {
        let data_dir = std::env::temp_dir().join(format!(
            "ai-assistant-persistence-test-{}",
            std::process::id()
        ));
        let _ = tokio::fs::remove_dir_all(&data_dir).await;
        set_data_dir(data_dir);

        let session_id = create_new_session("test-model").await.unwrap();
        let unique_content = format!("persist me in {}", session_id);
        let record = crate::ledger::LedgerRecord {
            record_id: 987,
            conversation_id: crate::ledger::DEFAULT_CONVERSATION_ID.to_string(),
            agent_id: crate::agent::keys::BOSS_AGENT_ID.to_string(),
            agent_name: "Boss".to_string(),
            role: crate::ledger::LedgerRole::User,
            content: unique_content.clone(),
            metadata: crate::ledger::LedgerMessageMeta::default(),
            created_at: "2026-04-30T00:00:00+00:00".to_string(),
        };

        append_ledger_record_current(&record).await.unwrap();
        let restored = load_full_session(&session_id).await.unwrap();

        let restored_record = restored
            .into_iter()
            .map(|message| message.into_ledger(99))
            .find(|record| record.content == unique_content)
            .expect("newly appended ledger record should be loadable");
        assert_eq!(restored_record.record_id, 987);
        assert_eq!(restored_record.role, crate::ledger::LedgerRole::User);
        assert_eq!(restored_record.agent_id, crate::agent::keys::BOSS_AGENT_ID);
    }
}

impl Default for ConversationIndex {
    fn default() -> Self {
        Self {
            version: 1,
            current_session: None,
            sessions: Vec::new(),
        }
    }
}

/// 单个会话的元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    /// 会话 ID（格式：sid_YYYYMMDD_HHMMSS）
    pub id: String,
    /// 会话标题（自动生成或用户自定义）
    pub title: String,
    /// 创建时间（ISO 8601）
    pub created_at: String,
    /// 最后更新时间（ISO 8601）
    pub updated_at: String,
    /// 消息条数
    pub message_count: u32,
    /// 使用的模型名称
    pub model: String,
    /// 该会话中创建过的子 Agent 快照
    #[serde(default)]
    pub agents: Vec<AgentSnapshot>,
    /// Boss 运行时导入的 feature Skills（恢复时重走 activate 路径自动计算 tools）
    #[serde(default)]
    pub imported_skills: Vec<String>,
    /// 当前执行计划（会话级恢复；None = 当前无计划）
    #[serde(default)]
    pub current_plan: Option<CurrentPlan>,
}

/// 子 Agent 在某次会话中的快照（用于恢复）
/// 精简版，对齐新式 AgentEntry（不再保存 tool_names/persona/log_file，
/// Agent 消息合并进会话 Ledger JSONL，通过 agent_id 字段区分来源）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    /// Agent ID（如 "agent_1"）
    pub id: String,
    /// 显示名
    pub name: String,
    /// 员工类型
    pub class: AgentClass,
    /// 最后记录的状态
    pub status: AgentStatus,
    /// 初始任务描述
    pub intent: String,
    /// 创建时的技能列表（main skills）
    pub skill_names: Vec<String>,
    /// 运行时动态导入的 feature Skills（恢复时重走 activate 路径自动计算 tools）
    #[serde(default)]
    pub imported_skills: Vec<String>,
    #[serde(default)]
    pub permissions: AgentPermissions,
}

/// 展示层元数据：仅用于前端渲染，不参与 LLM API 调用
/// `role: "tool"` 消息携带此字段时，前端将其渲染为 `tool_step` 气泡；
/// 额外的 `thinking_*` 字段用于还原 `thinking_step` 气泡（当前通过事件推送，
/// 此处预留以备未来将 thinking_step 也写入持久化）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayMeta {
    /// 前端气泡类型：`"tool_step"` | `"thinking_step"`
    pub display_role: String,
    /// tool_step：工具名称（如 `"ClickElement"`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// tool_step：完整命令字符串（如 `"ClickElement --selector #btn"`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_command: Option<String>,
    /// tool_step：执行是否成功
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// thinking_step：AI 推理过程文本
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// thinking_step：决策类型（`"executing"` / `"asking"` / `"result"`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// thinking_step：待执行工具列表（decision = "executing" 时有值）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// agent_report：复命来源 agent 的显示名（前端给气泡贴标签用）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
}

/// JSONL 持久化消息包装。新格式保存完整 LedgerRecord；读取时缺少
/// `ledger` 字段的旧记录会在 replay 时迁移为 LedgerRecord。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger: Option<LedgerRecord>,
    #[serde(flatten)]
    pub inner: Message,
    /// 来源 Agent ID；None = Boss 自身消息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// 展示层元数据；None = 正常 FC 协议消息，按 role 常规渲染
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayMeta>,
}

impl PersistedMessage {
    pub fn from_ledger(ledger: LedgerRecord) -> Self {
        Self {
            inner: Message {
                role: ledger.role.as_str().to_string(),
                content: ledger.content.clone(),
                cache_control: false,
                tool_call_id: None,
                name: None,
                tool_calls: None,
                reasoning_content: None,
            },
            agent_id: Some(ledger.agent_id.clone()),
            display: None,
            ledger: Some(ledger),
        }
    }

    pub fn into_ledger(self, _fallback_record_id: u64) -> LedgerRecord {
        if let Some(ledger) = self.ledger {
            ledger
        } else {
            LedgerRecord::from_persisted(_fallback_record_id, self)
        }
    }
}

/// 恢复结果
pub struct RestoreResult {
    /// 默认入口 Agent 对话历史
    pub default_agent_history: Vec<Message>,
    /// 需要重建的活跃子 Agent（仅 Interactive + Running/Idle）
    pub active_agents: Vec<AgentSnapshot>,
    /// 默认入口 Agent 上次会话的 imported_skills（恢复时传给 activate 路径自动计算 tools）
    pub imported_skills: Vec<String>,
    /// 默认入口 Agent 上次会话的当前计划
    pub current_plan: Option<CurrentPlan>,
}

// ============================================================================
// 路径工具
// ============================================================================

/// 获取会话存储根目录：`<data_dir>/conversations/`
/// 优先使用 `set_data_dir()` 注入的路径（由 Tauri 传入 app_local_data_dir）；
/// 未设置时回退到 `%LOCALAPPDATA%/Lumi/conversations/`（兼容旧行为）。
pub fn conversations_dir() -> PathBuf {
    if let Some(dir) = DATA_DIR.get() {
        return dir.join("conversations");
    }
    // 兼容回退
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("USERPROFILE").map(|u| format!("{}\\AppData\\Local", u)))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join("Lumi").join("conversations")
}

/// 生成会话 ID：`sid_YYYYMMDD_HHMMSS`
pub fn generate_session_id() -> String {
    let now = chrono::Local::now();
    format!("sid_{}", now.format("%Y%m%d_%H%M%S"))
}

/// 会话 Ledger 文件路径（新结构）
fn session_ledger_path(session_id: &str) -> PathBuf {
    conversations_dir()
        .join("sessions")
        .join(format!("{}.jsonl", session_id))
}

/// 旧版 Boss 会话文件路径（兼容读取/删除，不再写入）
fn legacy_boss_session_path(session_id: &str) -> PathBuf {
    conversations_dir()
        .join("boss")
        .join(format!("{}.jsonl", session_id))
}

/// 索引文件路径
fn index_path() -> PathBuf {
    conversations_dir().join("index.json")
}

/// 确保目录存在
async fn ensure_dirs() -> std::io::Result<()> {
    let base = conversations_dir();
    tokio::fs::create_dir_all(base.join("sessions")).await?;
    tokio::fs::create_dir_all(base.join("snapshots")).await?;
    Ok(())
}

// ============================================================================
// 全局当前 session_id（进程级单例）
// ============================================================================

use parking_lot::RwLock;
use std::sync::OnceLock;

/// 应用数据根目录（由外部在启动时 set，conversations/ 存放于此）
static DATA_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();

/// 由 Tauri 层（或其他宿主）在启动时调用，设置持久化根目录。
pub fn set_data_dir(dir: std::path::PathBuf) {
    let _ = DATA_DIR.set(dir);
}

static CURRENT_SESSION_ID: OnceLock<RwLock<String>> = OnceLock::new();
static DEFAULT_AGENT_ID: OnceLock<RwLock<String>> = OnceLock::new();

/// 获取当前 session_id
pub fn current_session_id() -> String {
    CURRENT_SESSION_ID
        .get()
        .map(|lock| lock.read().clone())
        .unwrap_or_default()
}

/// 设置当前 session_id
fn set_current_session_id(id: &str) {
    match CURRENT_SESSION_ID.get() {
        Some(lock) => {
            *lock.write() = id.to_string();
        }
        None => {
            let _ = CURRENT_SESSION_ID.set(RwLock::new(id.to_string()));
        }
    }
}

pub fn current_default_agent_id() -> String {
    DEFAULT_AGENT_ID
        .get()
        .map(|lock| lock.read().clone())
        .unwrap_or_else(|| crate::agent::keys::BOSS_AGENT_ID.to_string())
}

fn set_default_agent_id(id: &str) {
    let id = if id.trim().is_empty() {
        crate::agent::keys::BOSS_AGENT_ID
    } else {
        id.trim()
    };
    match DEFAULT_AGENT_ID.get() {
        Some(lock) => {
            *lock.write() = id.to_string();
        }
        None => {
            let _ = DEFAULT_AGENT_ID.set(RwLock::new(id.to_string()));
        }
    }
}

// ============================================================================
// 索引读写
// ============================================================================

/// 加载索引（不存在时返回默认空索引）
pub async fn load_index() -> crate::Result<ConversationIndex> {
    let path = index_path();
    if !path.exists() {
        return Ok(ConversationIndex::default());
    }
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| crate::Error::Persistence(format!("读取 index.json 失败: {}", e)))?;
    let index: ConversationIndex = serde_json::from_str(&content)
        .map_err(|e| crate::Error::Persistence(format!("解析 index.json 失败: {}", e)))?;
    Ok(index)
}

/// 保存索引
pub async fn save_index(index: &ConversationIndex) -> crate::Result<()> {
    ensure_dirs()
        .await
        .map_err(|e| crate::Error::Persistence(format!("创建目录失败: {}", e)))?;
    let content = serde_json::to_string_pretty(index)
        .map_err(|e| crate::Error::Persistence(format!("序列化 index.json 失败: {}", e)))?;
    tokio::fs::write(index_path(), content)
        .await
        .map_err(|e| crate::Error::Persistence(format!("写入 index.json 失败: {}", e)))?;
    Ok(())
}

// ============================================================================
// JSONL 读写
// ============================================================================

/// 追加一条 `PersistedMessage` 到 JSONL 文件
async fn append_jsonl(path: &PathBuf, msg: &PersistedMessage) -> crate::Result<()> {
    ensure_dirs()
        .await
        .map_err(|e| crate::Error::Persistence(format!("创建目录失败: {}", e)))?;
    let mut line = serde_json::to_string(msg)
        .map_err(|e| crate::Error::Persistence(format!("序列化消息失败: {}", e)))?;
    line.push('\n');

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| {
            crate::Error::Persistence(format!("打开 JSONL 失败 {}: {}", path.display(), e))
        })?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| crate::Error::Persistence(format!("写入 JSONL 失败: {}", e)))?;
    file.flush()
        .await
        .map_err(|e| crate::Error::Persistence(format!("flush JSONL 失败: {}", e)))?;
    Ok(())
}

/// 读取 JSONL 文件的所有持久化记录（保留 agent_id）
async fn read_jsonl_full(path: &PathBuf) -> crate::Result<Vec<PersistedMessage>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        crate::Error::Persistence(format!("读取 JSONL 失败 {}: {}", path.display(), e))
    })?;

    let mut records = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let pm = serde_json::from_str::<PersistedMessage>(trimmed).map_err(|e| {
            crate::Error::Persistence(format!(
                "解析 JSONL 失败 {} 第 {} 行: {}",
                path.display(),
                i + 1,
                e
            ))
        })?;
        records.push(pm);
    }
    Ok(records)
}

// ============================================================================
// 会话 Ledger 操作
// ============================================================================

pub async fn append_ledger_record_current(record: &LedgerRecord) -> crate::Result<()> {
    let sid = current_session_id();
    if sid.is_empty() {
        return Ok(());
    }
    let path = session_ledger_path(&sid);
    append_jsonl(&path, &PersistedMessage::from_ledger(record.clone())).await?;
    update_session_message_meta(&sid).await
}

async fn update_session_message_meta(session_id: &str) -> crate::Result<()> {
    let mut index = load_index().await?;
    if let Some(session) = index.sessions.iter_mut().find(|s| s.id == session_id) {
        session.message_count += 1;
        session.updated_at = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    }
    save_index(&index).await
}

async fn read_session_ledger_full(session_id: &str) -> crate::Result<Vec<PersistedMessage>> {
    let path = session_ledger_path(session_id);
    if path.exists() {
        return read_jsonl_full(&path).await;
    }

    let legacy_path = legacy_boss_session_path(session_id);
    read_jsonl_full(&legacy_path).await
}

fn session_ledger_exists(session_id: &str) -> bool {
    session_ledger_path(session_id).exists() || legacy_boss_session_path(session_id).exists()
}

/// 加载指定会话中属于默认 Agent 的消息（agent_id == None）
/// 子 Agent 消息虽然写在同一 JSONL 中，但此函数只返回默认 Agent 自身消息，
/// 确保默认 Agent 内存对话不混入子 Agent 的 FC 协议消息。
/// **过滤规则**：
/// - `agent_id.is_some()` → 子 Agent 消息，跳过
/// - `role == "display"` → 仅展示层占位（thinking_step / tool_step 气泡恢复用），
pub async fn load_default_agent_history(session_id: &str) -> crate::Result<Vec<Message>> {
    let default_agent_id = current_default_agent_id();
    let records = read_session_ledger_full(session_id).await?;
    Ok(records
        .into_iter()
        .filter(|pm| {
            pm.agent_id
                .as_deref()
                .map(|agent_id| {
                    agent_id == default_agent_id || agent_id == crate::agent::keys::BOSS_AGENT_ID
                })
                .unwrap_or(true)
        })
        .map(|pm| pm.inner)
        .filter(|msg| !matches!(msg.role.as_str(), "display" | "gateway_message"))
        .collect())
}

/// 兼容旧命名：默认 Agent 曾被称为 Boss。
pub async fn load_boss_session(session_id: &str) -> crate::Result<Vec<Message>> {
    load_default_agent_history(session_id).await
}

/// 加载指定会话中某个子 Agent 的消息（按 agent_id 过滤）
/// 返回完整 `PersistedMessage`（含 display 元数据），
/// 用于 `ai_agent_history` 在 Agent 不在内存时从磁盘恢复对话历史。
pub async fn load_agent_session(
    session_id: &str,
    agent_id: &str,
) -> crate::Result<Vec<PersistedMessage>> {
    let records = read_session_ledger_full(session_id).await?;
    Ok(records
        .into_iter()
        .filter(|pm| pm.agent_id.as_deref() == Some(agent_id))
        .collect())
}

/// 加载指定会话的全部消息（默认 Agent + 所有子 Agent，保持写入顺序）
/// 返回完整 `PersistedMessage`（含 display 元数据），
/// 用于 `ai_chat_history` 和 `switch_session` 返回完整对话流给前端。
pub async fn load_full_session(session_id: &str) -> crate::Result<Vec<PersistedMessage>> {
    read_session_ledger_full(session_id).await
}

/// 创建新会话（写入索引，切换 current_session）
pub async fn create_new_session(model: &str) -> crate::Result<String> {
    let sid = generate_session_id();
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();

    let meta = SessionMeta {
        id: sid.clone(),
        title: "新对话".to_string(),
        created_at: now.clone(),
        updated_at: now,
        message_count: 0,
        model: model.to_string(),
        agents: Vec::new(),
        imported_skills: Vec::new(),
        current_plan: None,
    };

    let mut index = load_index().await?;
    index.current_session = Some(sid.clone());
    index.sessions.push(meta);
    save_index(&index).await?;

    set_current_session_id(&sid);
    tracing::debug!(session_id = %sid, "session created");
    Ok(sid)
}

/// 归档当前会话（更新标题，清除 current_session 以便 init 创建新会话）
pub async fn archive_current_session(title: &str) -> crate::Result<()> {
    let sid = current_session_id();
    if sid.is_empty() {
        return Ok(());
    }

    let mut index = load_index().await?;
    if let Some(session) = index.sessions.iter_mut().find(|s| s.id == sid) {
        session.title = title.to_string();
        session.updated_at = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    }
    // 清除 current_session → init_persistence 下次会创建新会话
    index.current_session = None;
    save_index(&index).await?;

    set_current_session_id("");
    tracing::debug!(session_id = %sid, title = %title, "session archived");
    Ok(())
}

/// 切换到已有会话（返回该会话的 Boss 历史）
pub async fn set_current_session(session_id: &str) -> crate::Result<()> {
    let mut index = load_index().await?;

    // 验证目标会话存在
    if !index.sessions.iter().any(|s| s.id == session_id) {
        return Err(crate::Error::Persistence(format!(
            "会话 {} 不存在",
            session_id
        )));
    }

    index.current_session = Some(session_id.to_string());
    save_index(&index).await?;

    set_current_session_id(session_id);
    let history = Vec::<Message>::new();
    tracing::debug!(
        session_id = %session_id,
        message_count = history.len(),
        "session selected"
    );
    Ok(())
}

/// 重命名会话
pub async fn rename_session(session_id: &str, title: &str) -> crate::Result<()> {
    let mut index = load_index().await?;
    if let Some(session) = index.sessions.iter_mut().find(|s| s.id == session_id) {
        session.title = title.to_string();
    } else {
        return Err(crate::Error::Persistence(format!(
            "会话 {} 不存在",
            session_id
        )));
    }
    save_index(&index).await?;
    Ok(())
}

/// 删除会话（索引 + JSONL 文件 + 关联 Agent 日志）
pub async fn delete_session(session_id: &str) -> crate::Result<()> {
    let mut index = load_index().await?;

    // 不允许删除当前活跃会话
    if index.current_session.as_deref() == Some(session_id) {
        return Err(crate::Error::Persistence(
            "不能删除当前活跃的会话".to_string(),
        ));
    }

    // 找到并移除
    let removed = index.sessions.iter().position(|s| s.id == session_id);
    if let Some(pos) = removed {
        let _session = index.sessions.remove(pos);

        // 删除会话 Ledger（新路径 + 旧路径兼容）
        let ledger_path = session_ledger_path(session_id);
        if ledger_path.exists() {
            let _ = tokio::fs::remove_file(&ledger_path).await;
        }
        let legacy_path = legacy_boss_session_path(session_id);
        if legacy_path.exists() {
            let _ = tokio::fs::remove_file(&legacy_path).await;
        }
        // Agent 消息已合并进会话 Ledger JSONL，无独立消息文件需删除
    } else {
        return Err(crate::Error::Persistence(format!(
            "会话 {} 不存在",
            session_id
        )));
    }

    save_index(&index).await?;
    tracing::debug!(session_id = %session_id, "session deleted");
    Ok(())
}

/// 获取所有会话元数据列表
pub async fn list_sessions() -> crate::Result<Vec<SessionMeta>> {
    let index = load_index().await?;
    Ok(index.sessions)
}

// ============================================================================
// Agent 快照操作（新式 AgentEntry）
// ============================================================================

/// 注册子 Agent 到当前会话索引（Step D: CreateAgentSystem 调用）
pub async fn register_agent_to_session(snapshot: AgentSnapshot) -> crate::Result<()> {
    let sid = current_session_id();
    if sid.is_empty() {
        return Ok(());
    }

    let mut index = load_index().await?;
    if let Some(session) = index.sessions.iter_mut().find(|s| s.id == sid) {
        // 去重：同 id 的替换
        session.agents.retain(|a| a.id != snapshot.id);
        session.agents.push(snapshot);
    }
    save_index(&index).await?;
    Ok(())
}

/// 更新 Agent 状态到索引
pub async fn update_agent_status(agent_id: &str, status: &AgentStatus) -> crate::Result<()> {
    let sid = current_session_id();
    if sid.is_empty() {
        return Ok(());
    }

    let mut index = load_index().await?;
    if let Some(session) = index.sessions.iter_mut().find(|s| s.id == sid) {
        if let Some(agent) = session.agents.iter_mut().find(|a| a.id == agent_id) {
            agent.status = status.clone();
        }
    }
    save_index(&index).await?;
    Ok(())
}

/// 保存默认 Agent 的 imported_skills 到当前会话索引
/// UpdateSkills 执行后调用，确保 skill 状态持久化到磁盘。
/// tools 不存储——恢复时由 activate 路径自动从 skills 重新计算。
pub async fn save_default_agent_skill_state(imported_skills: Vec<String>) -> crate::Result<()> {
    let sid = current_session_id();
    if sid.is_empty() {
        return Ok(());
    }

    let mut index = load_index().await?;
    if let Some(session) = index.sessions.iter_mut().find(|s| s.id == sid) {
        session.imported_skills = imported_skills;
    }
    save_index(&index).await?;
    Ok(())
}

/// 兼容旧命名：默认 Agent 曾被称为 Boss。
pub async fn save_boss_skill_state(imported_skills: Vec<String>) -> crate::Result<()> {
    save_default_agent_skill_state(imported_skills).await
}

/// 保存默认 Agent 当前执行计划到当前会话索引
pub async fn save_default_agent_current_plan(
    current_plan: Option<CurrentPlan>,
) -> crate::Result<()> {
    let sid = current_session_id();
    if sid.is_empty() {
        return Ok(());
    }

    let mut index = load_index().await?;
    if let Some(session) = index.sessions.iter_mut().find(|s| s.id == sid) {
        session.current_plan = current_plan;
        session.updated_at = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    }
    save_index(&index).await?;
    Ok(())
}

/// 兼容旧命名：默认 Agent 曾被称为 Boss。
pub async fn save_boss_current_plan(current_plan: Option<CurrentPlan>) -> crate::Result<()> {
    save_default_agent_current_plan(current_plan).await
}

/// 读取指定会话的当前执行计划
pub async fn load_session_current_plan(session_id: &str) -> crate::Result<Option<CurrentPlan>> {
    let index = load_index().await?;
    Ok(index
        .sessions
        .iter()
        .find(|s| s.id == session_id)
        .and_then(|s| s.current_plan.clone()))
}

/// 保存子 Agent 的 imported_skills 到当前会话索引
/// 子 Agent 执行 UpdateSkills 后调用。
pub async fn save_agent_skill_state(
    agent_id: &str,
    imported_skills: Vec<String>,
) -> crate::Result<()> {
    let sid = current_session_id();
    if sid.is_empty() {
        return Ok(());
    }

    let mut index = load_index().await?;
    if let Some(session) = index.sessions.iter_mut().find(|s| s.id == sid) {
        if let Some(agent) = session.agents.iter_mut().find(|a| a.id == agent_id) {
            agent.imported_skills = imported_skills;
        }
    }
    save_index(&index).await?;
    Ok(())
}

// ============================================================================
// 恢复
// ============================================================================

/// 初始化持久化层：加载索引，恢复或创建当前会话
/// 返回需要恢复的默认入口 Agent 历史和活跃子 Agent 快照。
/// 调用方负责将 default_agent_history 写入 cache，将 active_agents 重建为 SubAgent。
pub async fn init_persistence(model: &str, default_agent_id: &str) -> crate::Result<RestoreResult> {
    set_default_agent_id(default_agent_id);
    ensure_dirs()
        .await
        .map_err(|e| crate::Error::Persistence(format!("创建目录失败: {}", e)))?;

    let mut index = load_index().await?;

    // 优先恢复上次的 current_session
    if let Some(ref sid) = index.current_session {
        if session_ledger_exists(sid) {
            set_current_session_id(sid);
            let default_agent_history = load_default_agent_history(sid).await?;

            // 从 SessionMeta 中读取默认 Agent 的 imported_skills + 筛选活跃子 Agent
            let session = index.sessions.iter().find(|s| s.id == *sid);

            let active_agents: Vec<AgentSnapshot> = session
                .map(|s| {
                    s.agents
                        .iter()
                        .filter(|a| {
                            matches!(a.class, AgentClass::Interactive) && a.status.is_active()
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();

            let imported_skills: Vec<String> = session
                .map(|s| s.imported_skills.clone())
                .unwrap_or_default();

            let current_plan: Option<CurrentPlan> = session.and_then(|s| s.current_plan.clone());

            tracing::info!(
                "恢复会话 {}：{}条默认 Agent 消息，{}个活跃 Agent，{}个 imported skills，计划={}",
                sid,
                default_agent_history.len(),
                active_agents.len(),
                imported_skills.len(),
                if current_plan.is_some() { "yes" } else { "no" },
            );

            return Ok(RestoreResult {
                default_agent_history,
                active_agents,
                imported_skills,
                current_plan,
            });
        }
    }

    // 无可恢复的会话 → 创建新会话
    let sid = create_new_session(model).await?;
    index.current_session = Some(sid.clone());
    set_current_session_id(&sid);

    Ok(RestoreResult {
        default_agent_history: Vec::new(),
        active_agents: Vec::new(),
        imported_skills: Vec::new(),
        current_plan: None,
    })
}

// ============================================================================
// 辅助工具
// ============================================================================

/// 从对话历史自动生成标题（取第一条 user 消息前 30 字符）
pub fn auto_title(history: &[Message]) -> String {
    history
        .iter()
        .find(|m| m.role == "user")
        .map(|m| {
            let content = m.content.trim();
            if content.chars().count() > 30 {
                format!("{}...", content.chars().take(30).collect::<String>())
            } else {
                content.to_string()
            }
        })
        .unwrap_or_else(|| "新对话".to_string())
}

/// 从 AgentEntry 字段构建快照（用于注册到索引）
pub fn snapshot_from_entry(
    id: &str,
    name: &str,
    class: AgentClass,
    skill_names: Vec<String>,
    intent: &str,
) -> AgentSnapshot {
    AgentSnapshot {
        id: id.to_string(),
        name: name.to_string(),
        class,
        status: AgentStatus::Running,
        intent: intent.to_string(),
        skill_names,
        imported_skills: Vec::new(), // 创建时无动态 skill，运行中由 UpdateSkills 写入
        permissions: AgentPermissions::default(),
    }
}

// ============================================================================
// Cache 快照（新式持久化：dump / restore）
// ============================================================================

/// cache 快照路径：`<conversations>/snapshots/<session_id>/<participant_id>.json`
/// `participant_id` 为配置或运行时创建的 Agent ID。
fn cache_snapshot_path(session_id: &str, participant_id: &str) -> PathBuf {
    conversations_dir()
        .join("snapshots")
        .join(session_id)
        .join(format!("{}.json", participant_id))
}

/// 旧版 cache 快照路径：`<conversations>/<session_id>/<participant_id>.json`
fn legacy_cache_snapshot_path(session_id: &str, participant_id: &str) -> PathBuf {
    conversations_dir()
        .join(session_id)
        .join(format!("{}.json", participant_id))
}

/// 确保 session 子目录存在
async fn ensure_session_dir(session_id: &str) -> std::io::Result<()> {
    tokio::fs::create_dir_all(conversations_dir().join("snapshots").join(session_id)).await
}

/// 将 ScopedCache 的全部字段导出为快照并写入磁盘
/// 文件路径：`<conversations>/snapshots/<session_id>/<participant_id>.json`
pub async fn save_cache_snapshot(
    session_id: &str,
    participant_id: &str,
    cache: &Arc<dyn corework::cache::Cache>,
) -> crate::Result<()> {
    let mut snapshot = cache
        .dump_raw()
        .await
        .map_err(|e| crate::Error::Persistence(format!("导出 cache 快照失败: {}", e)))?;

    snapshot.remove(crate::context::keys::HOST_DYNAMIC_SNAPSHOTS);

    ensure_session_dir(session_id)
        .await
        .map_err(|e| crate::Error::Persistence(format!("创建 session 目录失败: {}", e)))?;

    let path = cache_snapshot_path(session_id, participant_id);
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| crate::Error::Persistence(format!("序列化 cache 快照失败: {}", e)))?;
    tokio::fs::write(&path, json)
        .await
        .map_err(|e| crate::Error::Persistence(format!("写入 cache 快照失败: {}", e)))?;
    Ok(())
}

/// 从磁盘加载 cache 快照并批量写入 cache
/// 恢复后 cache 中包含 Agent 所有运行时字段，调用方只需根据
/// [`recovery_entry_from_messages`] 选择合适的初始状态启动状态机即可。
/// **Sanitize 规则**：
/// 1. 对 `conversation` / `compact_conversation` 字段过滤掉
///    `role == "display" / "thinking_step" / "tool_step" / "interrupted"
///    / "llm_error"` 的展示层占位消息（防御历史污染快照）。
/// 2. 调用 [`normalize_messages_value_for_recovery`] 截断对话尾部到稳态边界
///    （最后一条必须是 `user` 或不带 `tool_calls` 的 `assistant`），避免恢复成
///    “工具已发起但结果丢失”的脏中间态。
/// 3. 删除所有瞬态 `pending_*` / `next_state*` / `thinking_round_count` /
///    `pause_requested` / `waiting_for_input` 字段（见 [`RECOVERY_TRANSIENT_KEYS`]），
///    交由新生命周期的 `on_enter` 重新设置。
pub async fn restore_cache_snapshot(
    session_id: &str,
    participant_id: &str,
    cache: &Arc<dyn corework::cache::Cache>,
) -> crate::Result<bool> {
    let path = cache_snapshot_path(session_id, participant_id);
    let path = if path.exists() {
        path
    } else {
        let legacy_path = legacy_cache_snapshot_path(session_id, participant_id);
        if !legacy_path.exists() {
            return Ok(false);
        }
        legacy_path
    };

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| crate::Error::Persistence(format!("读取 cache 快照失败: {}", e)))?;
    let mut snapshot: std::collections::HashMap<String, serde_json::Value> =
        serde_json::from_str(&content)
            .map_err(|e| crate::Error::Persistence(format!("解析 cache 快照失败: {}", e)))?;

    // 过滤掉对话历史里残留的展示层占位消息（防御历史污染快照）
    snapshot.remove(crate::context::keys::HOST_DYNAMIC_SNAPSHOTS);

    for key in ["conversation", "compact_conversation"] {
        if let Some(val) = snapshot.get_mut(key) {
            if let Some(arr) = val.as_array_mut() {
                let before = arr.len();
                arr.retain(|m| {
                    let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
                    !matches!(
                        role,
                        "display" | "thinking_step" | "tool_step" | "interrupted" | "llm_error"
                    )
                });
                let removed = before - arr.len();
                if removed > 0 {
                    tracing::warn!(
                        "cache 快照清洗: {} 字段移除 {} 条展示层占位消息（防 invalid role）",
                        key,
                        removed
                    );
                }
            }
        }
    }

    // 截断对话尾部到 user / 干净 assistant 边界，避免“工具已发起但结果丢失”的脏中间态
    for key in ["conversation", "compact_conversation"] {
        if let Some(val) = snapshot.get_mut(key) {
            normalize_messages_value_for_recovery(val, key);
        }
    }

    // 丢弃所有瞬态字段，新一轮 on_enter 会按需重设
    for key in RECOVERY_TRANSIENT_KEYS {
        snapshot.remove(*key);
    }

    let items: Vec<(String, serde_json::Value)> = snapshot.into_iter().collect();
    cache
        .mset_raw(&items, None)
        .await
        .map_err(|e| crate::Error::Persistence(format!("恢复 cache 快照失败: {}", e)))?;

    tracing::debug!(
        "cache 快照已恢复: {} ({}个字段)",
        path.display(),
        items.len()
    );
    Ok(true)
}

// ============================================================================
// 恢复归一化（tool 调用未完成 / 末尾脏状态）
// ============================================================================

/// 恢复时需要丢弃的瞬态 cache 字段。
/// 这些字段都属于“某一具体 turn 的中间状态”，跨 pod / 跨进程恢复后没有意义；
/// 保留反而会让新生命周期误以为还有 pending tool / next_state 等待消费，
/// 产生“工具已发起但结果丢失”的幽灵执行。
pub const RECOVERY_TRANSIENT_KEYS: &[&str] = &[
    "pending_tools",
    "pending_structured_tools",
    "pending_tool_calls",
    "pending_tool_display_commands",
    "pending_tool_recovery_results",
    "pending_response",
    "pending_question",
    "pending_view",
    "pending_result",
    "retrieval_context",
    "retrieval_query_hash",
    "retrieval_turn_id",
    "retrieval_last_entry",
    "next_state",
    "next_state_after_saying",
    "thinking_round_count",
    "pause_requested",
    "waiting_for_input",
    // turn_id 是“当前 turn 的事件序号”，跨进程恢复后由新 thinking on_enter
    // 重新从 0 自增即可，前端凭 turn_id 过滤旧 turn 残余事件，恢复时本就需要清零。
    "turn_id",
];

/// 恢复后状态机应进入的稳态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryEntry {
    /// 最后一条是 `user`，需要直接进 thinking 跑 LLM。
    Thinking,
    /// 最后一条是干净 assistant 或对话为空，停在 suspended 等待用户输入。
    Suspended,
}

/// 判断 `Message` 是否为干净的可终结消息：
/// - `user`：用户消息，下一步应进 thinking
/// - `assistant`：必须没有 tool_calls，且 content 非空（避免空气泡终止）
fn is_clean_terminal_message(msg: &Message) -> bool {
    match msg.role.as_str() {
        "user" => true,
        "assistant" => {
            let no_tool_calls = msg
                .tool_calls
                .as_ref()
                .map(|v| v.is_empty())
                .unwrap_or(true);
            no_tool_calls && !msg.content.trim().is_empty()
        }
        _ => false,
    }
}

/// 同 [`is_clean_terminal_message`]，作用于 `serde_json::Value` 形式的消息。
fn is_clean_terminal_value(value: &serde_json::Value) -> bool {
    let role = value.get("role").and_then(|r| r.as_str()).unwrap_or("");
    match role {
        "user" => true,
        "assistant" => {
            let no_tool_calls = value
                .get("tool_calls")
                .map(|tc| tc.is_null() || tc.as_array().map(|a| a.is_empty()).unwrap_or(true))
                .unwrap_or(true);
            let content_empty = value
                .get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.trim().is_empty())
                .unwrap_or(true);
            no_tool_calls && !content_empty
        }
        _ => false,
    }
}

/// 截断对话尾部到稳态边界，返回截断后的列表。
/// 处理的脏尾巴包括：
/// - `assistant(tool_calls=...)`：模型决定调工具，但工具结果可能未完成
/// - `system` / 其它非终结角色
/// 截断后最后一条必为 `user` 或干净的 `assistant`；若整段都不可终结则返回空。
pub fn normalize_messages_for_recovery(messages: Vec<Message>) -> Vec<Message> {
    let mut trimmed = messages;
    while let Some(last) = trimmed.last() {
        if is_clean_terminal_message(last) {
            break;
        }
        trimmed.pop();
    }
    trimmed
}

/// 同上，但作用于 `serde_json::Value` 形式（用于 cache 快照 in-place 截断）。
fn normalize_messages_value_for_recovery(value: &mut serde_json::Value, key_name: &str) {
    let Some(arr) = value.as_array_mut() else {
        return;
    };
    let before = arr.len();
    while let Some(last) = arr.last() {
        if is_clean_terminal_value(last) {
            break;
        }
        arr.pop();
    }
    let removed = before - arr.len();
    if removed > 0 {
        tracing::warn!(
            "cache 快照恢复归一化: {} 字段截断 {} 条非终结消息（tool 调用未完成或脏尾）",
            key_name,
            removed
        );
    }
}

/// 根据归一化后的消息列表推断恢复入口状态。
pub fn recovery_entry_from_messages(messages: &[Message]) -> RecoveryEntry {
    match messages.last() {
        Some(msg) if msg.role == "user" => RecoveryEntry::Thinking,
        _ => RecoveryEntry::Suspended,
    }
}

/// 从 cache 中删除所有 [`RECOVERY_TRANSIENT_KEYS`] 字段。
/// 用于不走 [`restore_cache_snapshot`] 的恢复路径（如默认 Agent 仅从 ledger
/// 重建 conversation 时），确保旧 pod 残留的 pending tool / next_state 不会
/// 被新 pod 误读。
pub async fn clear_recovery_transient_keys(
    cache: &Arc<dyn corework::cache::Cache>,
) -> crate::Result<()> {
    for key in RECOVERY_TRANSIENT_KEYS {
        if let Err(e) = cache.delete(key).await {
            tracing::debug!("清理瞬态字段 {} 失败（可忽略）: {}", key, e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_session_id() {
        let id = generate_session_id();
        assert!(id.starts_with("sid_"));
        assert!(id.len() > 10);
    }

    #[test]
    fn test_auto_title_short() {
        let history = vec![Message::user("你好")];
        assert_eq!(auto_title(&history), "你好");
    }

    #[test]
    fn test_auto_title_long() {
        let long_msg = "这是一条非常非常长的消息，超过了三十个字符的限制需要截断处理呀";
        let history = vec![Message::user(long_msg)];
        let title = auto_title(&history);
        assert!(title.ends_with("..."));
        assert!(title.chars().count() <= 34); // 30 + "..."
    }

    #[test]
    fn test_auto_title_no_user_msg() {
        let history = vec![Message::assistant("你好")];
        assert_eq!(auto_title(&history), "新对话");
    }
    #[test]
    fn test_conversation_index_default() {
        let index = ConversationIndex::default();
        assert_eq!(index.version, 1);
        assert!(index.current_session.is_none());
        assert!(index.sessions.is_empty());
    }

    #[test]
    fn test_persisted_message_roundtrip() {
        let msg = Message::user("hello");
        let pm = PersistedMessage {
            ledger: None,
            inner: msg.clone(),
            agent_id: Some("agent_1".to_string()),
            display: None,
        };
        let json = serde_json::to_string(&pm).unwrap();
        let back: PersistedMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.inner.content, "hello");
        assert_eq!(back.agent_id.as_deref(), Some("agent_1"));
    }

    #[test]
    fn test_normalize_recovery_keeps_user_tail() {
        let history = vec![Message::user("hi"), Message::assistant("hello")];
        let trimmed = normalize_messages_for_recovery(history.clone());
        assert_eq!(trimmed.len(), 2);
        assert_eq!(
            recovery_entry_from_messages(&trimmed),
            RecoveryEntry::Suspended
        );

        let history2 = vec![Message::assistant("hello"), Message::user("again")];
        let trimmed2 = normalize_messages_for_recovery(history2);
        assert_eq!(trimmed2.len(), 2);
        assert_eq!(
            recovery_entry_from_messages(&trimmed2),
            RecoveryEntry::Thinking
        );
    }

    #[test]
    fn test_normalize_recovery_drops_dangling_tool() {
        // assistant(tool_calls=[...]) + tool(...)：tool 调用未必完成，应裁剪到上一条 user
        let mut assistant_with_tc = Message::assistant("");
        assistant_with_tc.tool_calls =
            Some(vec![llm_gateway::ToolCall::function("call_1", "Foo", "{}")]);
        let history = vec![
            Message::user("do it"),
            assistant_with_tc,
            Message::tool("partial result"),
        ];
        let trimmed = normalize_messages_for_recovery(history);
        assert_eq!(trimmed.len(), 1);
        assert_eq!(trimmed[0].role, "user");
        assert_eq!(
            recovery_entry_from_messages(&trimmed),
            RecoveryEntry::Thinking
        );
    }

    #[test]
    fn test_normalize_recovery_drops_empty_assistant() {
        let history = vec![Message::user("hi"), Message::assistant("")];
        let trimmed = normalize_messages_for_recovery(history);
        assert_eq!(trimmed.len(), 1);
        assert_eq!(trimmed[0].role, "user");
        assert_eq!(
            recovery_entry_from_messages(&trimmed),
            RecoveryEntry::Thinking
        );
    }

    #[test]
    fn test_normalize_recovery_empty_is_suspended() {
        let trimmed = normalize_messages_for_recovery(Vec::new());
        assert!(trimmed.is_empty());
        assert_eq!(
            recovery_entry_from_messages(&trimmed),
            RecoveryEntry::Suspended
        );
    }

    #[test]
    fn test_persisted_message_no_agent_id() {
        let msg = Message::user("boss msg");
        let pm = PersistedMessage {
            ledger: None,
            inner: msg,
            agent_id: None,
            display: None,
        };
        let json = serde_json::to_string(&pm).unwrap();
        // agent_id 字段不应出现在序列化结果中
        assert!(!json.contains("agent_id"));
    }
}
