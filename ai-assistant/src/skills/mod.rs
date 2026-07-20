//! 提供完整的 Skill 生命周期管理：
//! **注册** → **发现** → **加载** → **构建提示词**
//! ## 架构
//! ```text
//! skills.json (注册表)
//!     │
//!     ▼
//! SkillManager
//!     ├─ discover()      → 扫描目录，收集元数据 (Tier 1)
//!     ├─ load(name)      → 按需加载完整 Skill (Tier 2)
//!     ├─ catalog_prompt() → 生成技能目录提示词
//!     └─ skill_prompt(name) → 生成已触发技能的指导提示词
//！ skills_dir/
//!     ├─ skill-a/
//!     │   ├─ SKILL.md
//!     │   ├─ reference/
//!     │   └─ scripts/
//!     └─ skill-b/
//!         └─ SKILL.md
//! ```

pub mod embedded;
pub mod loader;
pub mod prompt;
pub mod systems;
pub mod types;

pub use systems::{init_skill_manager, replace_skill_manager, system_names};
pub use types::{Skill, SkillEntry, SkillGroup, SkillLoadState, SkillMetadata, SkillRegistry};

use corework::error::{FrameworkError, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Skill 管理器
/// 核心职责：
/// 1. 从 skills.json 读取注册表配置
/// 3. 按名称动态加载 Skill 内容
/// 4. 构建提示词（catalog / instruction）
/// ## 三层架构
/// 支持独立的三层 Skill 目录，各有自己的 skills.json：
/// - **system/**：状态行为规范（`system_layer: true`，不对 AI 展示）
/// - **main/**：Agent 身份定义（构建时确定，不可运行时切换）
/// - **feature/**：功能技能（按需激活/卸载）
/// 所有层的 entries 合并到同一个 HashMap 中，通过 `system_layer` 字段区分。
pub struct SkillManager {
    /// Skills 根目录（feature 层，兼容旧路径）
    skills_dir: PathBuf,
    system_dir: Option<PathBuf>,
    /// 基础层目录（可选）
    main_dir: Option<PathBuf>,
    /// 注册表配置（可选，无配置时扫描全目录）
    registry: Option<SkillRegistry>,
    /// 运行时 Skill 条目索引（name → SkillEntry）
    entries: HashMap<String, SkillEntry>,
}

impl SkillManager {
    /// 创建空的 SkillManager
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        Self {
            skills_dir: skills_dir.into(),
            system_dir: None,
            main_dir: None,
            registry: None,
            entries: HashMap::new(),
        }
    }

    /// 创建预填充内嵌 system 层的 SkillManager（推荐使用）
    pub fn new_with_embedded_system(skills_dir: impl Into<PathBuf>) -> Self {
        let mut entries = HashMap::new();
        for (name, entry) in embedded::embedded_system_entries() {
            entries.insert(name, entry);
        }
        Self {
            skills_dir: skills_dir.into(),
            system_dir: None, // system 层已内嵌，不走文件系统
            main_dir: None,
            registry: None,
            entries,
        }
    }

    /// 从注册表文件初始化（skills.json + 目录扫描）
    /// 自动探测三层目录结构：
    /// - 若 `skills_dir` 下存在 `system/`、`main/`、`feature/` 子目录，使用三层模式
    /// - 否则回退到单层模式（全部在 `skills_dir` 下）
    pub async fn from_registry(registry_path: &Path) -> Result<Self> {
        let registry = loader::load_registry(registry_path).await?;
        let skills_dir = if registry.skills_dir.is_relative() {
            registry_path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&registry.skills_dir)
        } else {
            registry.skills_dir.clone()
        };

        tracing::debug!(
            "from_registry: registry_path={:?}, skills_dir={:?}",
            registry_path,
            skills_dir
        );

        // 探测三层目录
        let system_dir = skills_dir.join("system");
        let role_dir = skills_dir.join("role");
        let legacy_main_dir = skills_dir.join("main");
        let main_dir = if role_dir.is_dir() {
            role_dir
        } else {
            legacy_main_dir
        };
        let feature_dir = skills_dir.join("feature");

        let has_layers = system_dir.is_dir() || main_dir.is_dir() || feature_dir.is_dir();

        let mut manager = Self::new_with_embedded_system(if has_layers {
            feature_dir.clone()
        } else {
            skills_dir.clone()
        });
        manager.system_dir = None; // 内嵌替代文件系统 system 层
        manager.main_dir = if main_dir.is_dir() {
            Some(main_dir)
        } else {
            None
        };
        manager.registry = Some(registry);

        manager.discover().await?;
        Ok(manager)
    }

    /// 从目录初始化（无 registry，扫描全目录）
    pub async fn from_directory(skills_dir: &Path) -> Result<Self> {
        let system_dir = skills_dir.join("system");
        let role_dir = skills_dir.join("role");
        let legacy_main_dir = skills_dir.join("main");
        let main_dir = if role_dir.is_dir() {
            role_dir
        } else {
            legacy_main_dir
        };
        let feature_dir = skills_dir.join("feature");
        let has_layers = system_dir.is_dir() || main_dir.is_dir() || feature_dir.is_dir();

        let mut manager = Self::new_with_embedded_system(if has_layers {
            feature_dir
        } else {
            skills_dir.to_path_buf()
        });
        manager.system_dir = None; // 内嵌替代文件系统 system 层
        manager.main_dir = if main_dir.is_dir() {
            Some(main_dir)
        } else {
            None
        };
        manager.discover().await?;
        Ok(manager)
    }

    // ========================================================================
    // 发现
    // ========================================================================

    /// 扫描目录发现所有 Skill（Tier 1: 仅元数据）
    /// 三层架构下依次扫描 system → main → feature（skills_dir）
    pub async fn discover(&mut self) -> Result<()> {
        // (system_dir 字段保留用于兼容旧路径，但 from_registry/from_directory 已设为 None)
        if let Some(ref sys_dir) = self.system_dir {
            let sys_registry_path = sys_dir.join("skills.json");
            let sys_registry = if sys_registry_path.is_file() {
                Some(loader::load_registry(&sys_registry_path).await?)
            } else {
                None
            };
            let discovered = loader::discover_skills(sys_dir).await?;
            tracing::debug!("系统层发现 {} 个 skills", discovered.len());
            for (dir_name, _meta) in discovered {
                if let Some(ref reg) = sys_registry {
                    if !reg.contains(&dir_name) {
                        continue;
                    }
                }
                let skill_file = sys_dir.join(&dir_name).join("SKILL.md");
                match loader::load_skill_file(&skill_file).await {
                    Ok(skill) => {
                        self.entries.insert(
                            dir_name.clone(),
                            SkillEntry {
                                name: dir_name,
                                state: SkillLoadState::FullyLoaded,
                                skill: Some(skill),
                            },
                        );
                    }
                    Err(e) => {
                        tracing::warn!("系统层 skill '{}' 完整加载失败: {}", dir_name, e);
                    }
                }
            }
        }

        // ---- 基础层 ----
        if let Some(ref main_dir) = self.main_dir {
            let main_registry_path = main_dir.join("skills.json");
            let main_registry = if main_registry_path.is_file() {
                Some(loader::load_registry(&main_registry_path).await?)
            } else {
                None
            };
            let discovered = loader::discover_skills(main_dir).await?;
            tracing::debug!("基础层发现 {} 个 skills", discovered.len());
            for (dir_name, meta) in discovered {
                if let Some(ref reg) = main_registry {
                    if !reg.contains(&dir_name) {
                        continue;
                    }
                }
                let base_path = main_dir.join(&dir_name);
                self.entries.insert(
                    dir_name.clone(),
                    SkillEntry {
                        name: dir_name,
                        state: SkillLoadState::MetadataLoaded,
                        skill: Some(Skill {
                            metadata: meta,
                            instructions: String::new(),
                            base_path: Some(base_path),
                        }),
                    },
                );
            }
        }

        // ---- 功能层（skills_dir）----
        let discovered = loader::discover_skills(&self.skills_dir).await?;
        tracing::debug!("功能层发现 {} 个 skills 目录", discovered.len());

        for (dir_name, meta) in discovered {
            // 如果有顶层 registry，仅加载已注册的
            if let Some(ref reg) = self.registry {
                if !reg.contains(&dir_name) {
                    tracing::debug!("跳过未注册的 skill: {}", dir_name);
                    continue;
                }
            }
            // 检查 feature 层自己的 skills.json
            let feature_registry_path = self.skills_dir.join("skills.json");
            if feature_registry_path.is_file() {
                if let Ok(feat_reg) = loader::load_registry(&feature_registry_path).await {
                    if !feat_reg.contains(&dir_name) {
                        tracing::debug!("跳过功能层未注册的 skill: {}", dir_name);
                        continue;
                    }
                }
            }

            tracing::debug!("加载 skill 元数据: {}", dir_name);
            let base_path = self.skills_dir.join(&dir_name);
            self.entries.insert(
                dir_name.clone(),
                SkillEntry {
                    name: dir_name.clone(),
                    state: SkillLoadState::MetadataLoaded,
                    skill: Some(Skill {
                        metadata: meta,
                        instructions: String::new(),
                        base_path: Some(base_path),
                    }),
                },
            );
        }

        // 注册表中声明但目录不存在的 → 标记为 Registered
        if let Some(ref reg) = self.registry {
            for name in reg.all_skill_names() {
                if !self.entries.contains_key(name) {
                    tracing::debug!("标记为已注册但未发现: {}", name);
                    self.entries.insert(
                        name.to_string(),
                        SkillEntry {
                            name: name.to_string(),
                            state: SkillLoadState::Registered,
                            skill: None,
                        },
                    );
                }
            }
        }

        tracing::debug!(
            "SkillManager 初始化完成，共 {} 个 entries",
            self.entries.len()
        );
        Ok(())
    }

    // ========================================================================
    // 加载
    // ========================================================================

    /// 按名称加载完整 Skill（Tier 1 + Tier 2）
    /// 幂等操作：已完全加载的不会重复读取。
    /// 自动在三层目录（system → main → feature）中查找。
    pub async fn load(&mut self, name: &str) -> Result<&Skill> {
        // 检查是否已完全加载
        if self.is_fully_loaded(name) {
            return self
                .entries
                .get(name)
                .and_then(|e| e.skill.as_ref())
                .ok_or_else(|| {
                    FrameworkError::InvalidData(format!(
                        "Skill '{}' 状态异常：FullyLoaded 但无数据",
                        name
                    ))
                });
        }

        // 依次在 system → main → feature 目录中查找
        let skill = self.load_from_layers(name).await?;

        self.entries.insert(
            name.to_string(),
            SkillEntry {
                name: name.to_string(),
                state: SkillLoadState::FullyLoaded,
                skill: Some(skill),
            },
        );

        self.entries
            .get(name)
            .and_then(|e| e.skill.as_ref())
            .ok_or_else(|| {
                FrameworkError::InvalidData(format!("Skill '{}' 刚加载但无法获取", name))
            })
    }

    /// 在三层目录中查找并加载 Skill
    async fn load_from_layers(&self, name: &str) -> Result<Skill> {
        // 优先检查 system 层
        if let Some(ref sys_dir) = self.system_dir {
            let skill_file = sys_dir.join(name).join("SKILL.md");
            if skill_file.exists() {
                return loader::load_skill_file(&skill_file).await;
            }
        }
        // 其次检查 main 层
        if let Some(ref main_dir) = self.main_dir {
            let skill_file = main_dir.join(name).join("SKILL.md");
            if skill_file.exists() {
                return loader::load_skill_file(&skill_file).await;
            }
        }
        // 最后检查 feature 层（skills_dir）
        loader::load_skill_by_name(&self.skills_dir, name).await
    }

    /// 批量加载多个 Skills
    pub async fn load_many(&mut self, names: &[&str]) -> Result<Vec<&Skill>> {
        // 先加载所有（需要 &mut self）
        for name in names {
            if !self.is_fully_loaded(name) {
                let skill = self.load_from_layers(name).await?;
                self.entries.insert(
                    name.to_string(),
                    SkillEntry {
                        name: name.to_string(),
                        state: SkillLoadState::FullyLoaded,
                        skill: Some(skill),
                    },
                );
            }
        }

        // 再收集引用
        let mut result = Vec::new();
        for name in names {
            if let Some(skill) = self.get(name) {
                result.push(skill);
            } else {
                tracing::warn!("无法获取 skill '{}'", name);
            }
        }
        Ok(result)
    }

    // ========================================================================
    // 查询
    // ========================================================================

    /// 获取已加载的 Skill（不触发文件 IO）
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.entries.get(name).and_then(|e| e.skill.as_ref())
    }

    /// 获取所有已发现 Skill 的元数据列表（含所有层）
    pub fn all_metadata(&self) -> Vec<&SkillMetadata> {
        self.entries
            .values()
            .filter_map(|e| e.skill.as_ref().map(|s| &s.metadata))
            .collect()
    }

    /// 获取 feature 层的 Skill 元数据列表（排除 system 层和 main 层）
    /// 用于 DiscoverSkills 等面向 AI 的接口，只展示可按需激活的功能技能。
    pub fn feature_metadata(&self) -> Vec<&SkillMetadata> {
        let main_names: std::collections::HashSet<String> =
            self.main_skill_names().into_iter().collect();
        self.entries
            .values()
            .filter_map(|e| e.skill.as_ref())
            .filter(|s| !s.metadata.system_layer && !main_names.contains(&s.metadata.name))
            .map(|s| &s.metadata)
            .collect()
    }

    /// 获取所有已注册的 Skill 名称
    pub fn all_names(&self) -> Vec<&str> {
        self.entries.keys().map(|s| s.as_str()).collect()
    }

    /// 获取 main 层（基础层）的所有 Skill 名称
    /// main 层 skill 在 Agent 初始化时自动激活，相当于 Agent 的身份定义。
    pub fn main_skill_names(&self) -> Vec<String> {
        if let Some(ref main_dir) = self.main_dir {
            self.entries
                .values()
                .filter_map(|e| e.skill.as_ref())
                .filter(|s| {
                    s.base_path
                        .as_ref()
                        .map(|p| p.starts_with(main_dir))
                        .unwrap_or(false)
                })
                .map(|s| s.metadata.name.clone())
                .collect()
        } else {
            vec![]
        }
    }

    /// 判断是否已完全加载
    fn is_fully_loaded(&self, name: &str) -> bool {
        self.entries
            .get(name)
            .map(|e| e.state == SkillLoadState::FullyLoaded)
            .unwrap_or(false)
    }

    /// 获取加载状态
    pub fn load_state(&self, name: &str) -> Option<SkillLoadState> {
        self.entries.get(name).map(|e| e.state)
    }

    /// 条目数量
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // ========================================================================
    // 提示词构建
    // ========================================================================

    /// 生成技能目录提示词（Tier 1 - 所有 Skill 的 name + description）
    pub fn catalog_prompt(&self) -> String {
        let metas: Vec<SkillMetadata> = self
            .all_metadata()
            .into_iter()
            .filter(|m| !m.system_layer) // 过滤系统层
            .cloned()
            .collect();
        prompt::build_skills_catalog(&metas)
    }

    /// 生成已触发 Skill 的指导提示词（Tier 2）
    /// 需要先调用 `load(name)` 确保已完全加载
    pub fn skill_prompt(&self, name: &str) -> Option<String> {
        self.get(name)
            .filter(|s| !s.instructions.is_empty())
            .map(|s| prompt::build_skill_prompt(s))
    }

    /// 生成多个已触发 Skills 的合并提示词
    pub fn active_skills_prompt(&self, names: &[&str]) -> String {
        let skills: Vec<&Skill> = names
            .iter()
            .filter_map(|n| self.get(n))
            .filter(|s| !s.instructions.is_empty())
            .collect();

        if skills.is_empty() {
            return String::new();
        }

        let owned: Vec<Skill> = skills.into_iter().cloned().collect();
        prompt::build_active_skills_prompt(&owned)
    }

    /// 获取指定状态的行为指示文本
    /// 每个状态对应 `system/{state}/SKILL.md` 的 instructions body。
    /// 若对应 Skill 未加载或 instructions 为空，返回 `None`。
    pub fn get_state_instruction(&self, state: &str) -> Option<String> {
        let skill = self.get(state)?;
        if !skill.metadata.system_layer {
            return None; // 非系统层 skill 不作为状态指示
        }
        let text = skill.instructions.trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    }

    /// 根据状态 skill 的 `tool_filter` 声明过滤工具列表
    /// 流程：
    /// 1. 读取 system skill 的 `tools` 字段，追加到列表（去重）
    /// 2. 读取 `tool_filter` 字段决定过滤策略：
    ///    - `"readonly"` → 只保留 inventory 中 `readonly=true` 的工具，但 `tools` 声明的豁免
    ///    - `"all"` / `None` → 不过滤，原样返回
    pub fn filtered_tools_for_state(&self, state: &str, mut tools: Vec<String>) -> Vec<String> {
        let skill = match self.get(state) {
            Some(s) if s.metadata.system_layer => s,
            _ => return tools, // 非系统层或不存在，不过滤
        };

        // 1. 追加 system skill 声明的工具（去重）
        let state_tools = &skill.metadata.tools;
        self.inject_tools_for_state(state, &mut tools);

        // 2. 按 tool_filter 过滤
        let filter = skill.metadata.tool_filter.as_deref().unwrap_or("all");
        match filter {
            "readonly" => {
                let all_factories: Vec<&corework::ai_system::AISystemFactory> =
                    inventory::iter::<corework::ai_system::AISystemFactory>
                        .into_iter()
                        .collect();
                tools.retain(|name| {
                    // system skill 声明的工具直接放行
                    if state_tools.contains(name) {
                        return true;
                    }
                    // 其余只保留 readonly=true
                    all_factories
                        .iter()
                        .find(|f| f.metadata.name == name.as_str())
                        .map(|f| f.metadata.readonly)
                        .unwrap_or(false)
                });
            }
            _ => {
                // "all" 或其他值：不过滤
            }
        }

        tools.sort();
        tools.dedup();
        tools
    }

    /// 将指定 system state Skill 声明的工具加入 Agent 基础白名单。
    pub fn inject_tools_for_state(&self, state: &str, tools: &mut Vec<String>) {
        let Some(skill) = self.get(state).filter(|skill| skill.metadata.system_layer) else {
            return;
        };
        for tool in &skill.metadata.tools {
            if !tools.contains(tool) {
                tools.push(tool.clone());
            }
        }
    }

    // ========================================================================
    // Reference（Tier 3）
    // ========================================================================

    /// 加载 Skill 的 reference 文件
    pub async fn load_reference(&self, skill_name: &str, filename: &str) -> Result<String> {
        let skill = self
            .get(skill_name)
            .ok_or_else(|| FrameworkError::InvalidData(format!("Skill '{}' 未加载", skill_name)))?;

        loader::load_reference(skill, filename).await
    }

    /// 将指定 skills 的工具注入到 tools 列表（去重追加）
    pub fn inject_tools_for_skills(&self, skill_names: &[&str], tools: &mut Vec<String>) {
        for name in skill_names {
            if let Some(skill) = self.get(name) {
                for t in &skill.metadata.tools {
                    if !tools.contains(t) {
                        tools.push(t.clone());
                    }
                }
            }
        }
    }

    /// 将三层（system + main + feature）所有 skills 的工具注入到 tools 列表（去重追加）
    pub fn inject_all_layer_tools(&self, tools: &mut Vec<String>) {
        for entry in self.entries.values() {
            if let Some(skill) = &entry.skill {
                for t in &skill.metadata.tools {
                    if !tools.contains(t) {
                        tools.push(t.clone());
                    }
                }
            }
        }
    }

    /// 收集指定 skills 关联的所有工具名称（从 metadata.tools 提取，去重）
    pub fn collect_tools_for_skills(&self, skill_names: &[&str]) -> Vec<String> {
        let mut tools = Vec::new();
        self.inject_tools_for_skills(skill_names, &mut tools);
        tools
    }

    /// 收集所有已发现 skills 的工具名称（去重）
    pub fn collect_all_tools(&self) -> Vec<String> {
        let mut tools = Vec::new();
        self.inject_all_layer_tools(&mut tools);
        tools
    }

    /// 收集指定 skills 关联的所有工作流名称（从 metadata.workflows 提取，去重）
    pub fn collect_workflows_for_skills(&self, skill_names: &[&str]) -> Vec<String> {
        let mut workflows = Vec::new();
        for name in skill_names {
            if let Some(skill) = self.get(name) {
                for wf in &skill.metadata.workflows {
                    if !workflows.contains(wf) {
                        workflows.push(wf.clone());
                    }
                }
            }
        }
        workflows
    }

    /// 收集所有已发现 skills 的工作流名称（去重）
    pub fn collect_all_workflows(&self) -> Vec<String> {
        let mut workflows = Vec::new();
        for entry in self.entries.values() {
            if let Some(skill) = &entry.skill {
                for wf in &skill.metadata.workflows {
                    if !workflows.contains(wf) {
                        workflows.push(wf.clone());
                    }
                }
            }
        }
        workflows
    }

    /// 获取注册表（如果有）
    pub fn registry(&self) -> Option<&SkillRegistry> {
        self.registry.as_ref()
    }

    /// 列出 Skill 的所有 reference 文件
    pub async fn list_references(&self, skill_name: &str) -> Result<Vec<String>> {
        let skill = self
            .get(skill_name)
            .ok_or_else(|| FrameworkError::InvalidData(format!("Skill '{}' 未加载", skill_name)))?;

        loader::list_references(skill).await
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_skill_manager_new() {
        let mgr = SkillManager::new("/tmp/skills");
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn thinking_tools_are_available_for_the_agent_base_whitelist() {
        let mgr = SkillManager::new_with_embedded_system("/tmp/skills");
        let mut tools = vec!["ContinueThinking".to_string()];

        mgr.inject_tools_for_state("thinking", &mut tools);

        assert!(tools.contains(&"GetSkillsList".to_string()));
        assert!(tools.contains(&"UpdateSkills".to_string()));
        assert!(tools.contains(&"PlanWrite".to_string()));
        assert!(tools.contains(&"PlanUpdate".to_string()));
        assert!(tools.contains(&"PlanFinish".to_string()));
        assert_eq!(
            tools
                .iter()
                .filter(|tool| tool.as_str() == "ContinueThinking")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn test_discover_nonexistent_dir() {
        let mut mgr = SkillManager::new("/nonexistent/path/skills");
        let result = mgr.discover().await;
        assert!(result.is_ok()); // 不存在目录返回空，不报错
        assert!(mgr.is_empty());
    }

    /// 验证 feature_metadata() 只返回 feature 层，不含 main 层
    #[tokio::test]
    async fn test_feature_metadata_excludes_main() {
        // 使用 crate 内的 skills 目录。
        // 当前仓库仅内嵌 system 层，没有独立 feature 目录时，feature_metadata 允许为空。
        let skills_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");

        tracing::debug!("skills_dir = {:?}", skills_dir);
        assert!(skills_dir.exists(), "skills 目录不存在: {:?}", skills_dir);

        let mgr = SkillManager::from_directory(&skills_dir)
            .await
            .expect("from_directory 失败");

        let all: Vec<String> = mgr.all_metadata().iter().map(|m| m.name.clone()).collect();
        let feature: Vec<String> = mgr
            .feature_metadata()
            .iter()
            .map(|m| m.name.clone())
            .collect();
        let main_names: Vec<String> = mgr.main_skill_names();

        tracing::debug!("all_metadata     : {:?}", all);
        tracing::debug!("feature_metadata : {:?}", feature);
        tracing::debug!("main_skill_names : {:?}", main_names);

        // main 层的名字不应出现在 feature_metadata 里
        for name in &main_names {
            assert!(
                !feature.contains(name),
                "main skill '{}' 不应出现在 feature_metadata 里，但出现了。feature={:?}",
                name,
                feature
            );
        }

        // 若仓库未提供 feature 层，feature_metadata 为空是合法的；这里主要验证“不混入 main 层”。
    }
}
