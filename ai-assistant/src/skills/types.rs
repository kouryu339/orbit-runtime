//! Skill 类型定义
//! 定义 Skill 标准文件结构与配置注册表的数据模型

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ============================================================================
// 核心类型
// ============================================================================

/// Skill 元数据（Tier 1 - 始终在上下文中）
/// 来自 SKILL.md 的 YAML frontmatter，轻量级，
/// 仅用于 AI 判断何时触发该 Skill（~100 词）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// 唯一标识符（kebab-case，如 "excel-processing"）
    pub name: String,

    /// 触发描述——**最重要的字段**
    /// AI 根据此字段决定是否触发 Skill，必须包含：
    /// - 适用场景（"Use when..."）
    /// - 触发关键词
    /// - 反例排除（"Do NOT trigger when..."）
    pub description: String,

    /// 这些工具会自动加入 `default_tools`，其描述注入提示词。
    /// 例如：`["Calculator", "TextRepeater"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,

    /// 关联工作流（工作流名称列表）
    /// 该 Skill 绑定的工作流名称。当 Skill 被激活时，
    /// 这些工作流会从注册表中筛出并注入 system prompt 的「已注册工作流」章节。
    /// 例如：`["每日签到", "批量下载"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflows: Vec<String>,

    /// 许可证说明（可选）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    /// - 不出现在 `catalog_prompt` 目录中（AI 无法手动激活）
    /// - 始终随 `default` 分组一起加载
    /// - 通过 `SkillManager::get_state_instruction()` 提取状态指令
    #[serde(default)]
    pub system_layer: bool,

    /// 工具过滤策略（仅 system_layer skill 有效）
    /// 控制该状态下 AI 可使用的工具范围：
    /// - `"all"`（默认）：不过滤，所有 ACTIVE_TOOLS 全部可用
    /// - `"readonly"`：只保留 `readonly=true` 的工具，但 `tools` 字段声明的工具豁免
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_filter: Option<String>,

    /// Skill 类别
    /// - `"role"`：角色定义型 skill（如 role/navigation 是 Boss 的人设）。
    ///   每个 Agent **必须且只能有一个 role skill**，避免无身份或多重人格。
    /// - `"capability"`（默认）：能力型 skill，描述工具/操作手册，可叠加多个。
    /// 该字段同时驱动后端两层约束：
    /// 1. `thinking` 只从 role skill 构建 persona，不再使用 default persona；
    /// 2. `CreateAgent` 要求传入的 skills 中必须且只能有一个 role skill。
    #[serde(
        default = "default_skill_kind",
        skip_serializing_if = "Option::is_none"
    )]
    pub kind: Option<String>,
}

fn default_skill_kind() -> Option<String> {
    Some("capability".to_string())
}

impl SkillMetadata {
    /// 是否为角色定义型 skill
    pub fn is_role(&self) -> bool {
        matches!(self.kind.as_deref(), Some("role"))
    }
}

/// 完整 Skill 定义（Tier 1 + Tier 2）
/// 包含元数据和详细指导内容（instructions body）
/// ## 文件格式（兼容 MCP Agent Skills）
/// ```markdown
/// ---
/// name: skill-identifier
/// description: "Detailed description with triggers..."
/// license: Optional license text
/// ---
/// # Step-by-step instructions in Markdown
/// ...
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// 元数据（YAML frontmatter）
    pub metadata: SkillMetadata,

    /// 详细指导内容（Markdown body，Tier 2）
    /// 触发后才加载到 prompt 中，<5k 词推荐
    pub instructions: String,

    /// Skill 文件所在目录（用于解析 reference/scripts 相对路径）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_path: Option<PathBuf>,
}

impl Skill {
    /// 获取 name 的便捷方法
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    /// 获取 description 的便捷方法
    pub fn description(&self) -> &str {
        &self.metadata.description
    }

    /// 获取 reference 目录路径（如果存在）
    pub fn reference_dir(&self) -> Option<PathBuf> {
        self.base_path.as_ref().map(|p| p.join("reference"))
    }

    /// 获取 scripts 目录路径（如果存在）
    pub fn scripts_dir(&self) -> Option<PathBuf> {
        self.base_path.as_ref().map(|p| p.join("scripts"))
    }
}

// ============================================================================
// 注册表配置（JSON 配置文件）
// ============================================================================

/// Skill 注册表——配置 JSON 的根结构
/// ## 配置文件格式（skills.json）
/// ```json
/// {
///   "skills_dir": "./skills",
///   "groups": [
///     {
///       "name": "document-skills",
///       "description": "文档处理技能集",
///       "skills": ["xlsx", "docx", "pptx"]
///     }
///   ]
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRegistry {
    pub skills_dir: PathBuf,

    /// Skill 分组列表
    #[serde(default)]
    pub groups: Vec<SkillGroup>,
}

/// Skill 条目（name = skill 目录名）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillGroup {
    /// Skill 目录名（同时作为唯一标识）
    pub name: String,

    /// 描述
    #[serde(default)]
    pub description: String,
}

impl SkillRegistry {
    /// 获取所有已注册的 skill 名称（去重）
    pub fn all_skill_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.groups.iter().map(|g| g.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// 判断某 skill 是否已注册
    pub fn contains(&self, name: &str) -> bool {
        self.groups.iter().any(|g| g.name == name)
    }
}

// ============================================================================
// 加载状态追踪
// ============================================================================

/// Skill 加载状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillLoadState {
    /// 已注册但未加载（仅知道 name）
    Registered,
    /// 元数据已加载（Tier 1: name + description）
    MetadataLoaded,
    /// 完整加载（Tier 1 + Tier 2: 含 instructions）
    FullyLoaded,
}

/// 运行时 Skill 条目
#[derive(Debug, Clone)]
pub struct SkillEntry {
    /// Skill 名称
    pub name: String,
    /// 当前加载状态
    pub state: SkillLoadState,
    /// 完整 Skill 数据（FullyLoaded 后可用）
    pub skill: Option<Skill>,
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_metadata_serde() {
        let meta = SkillMetadata {
            name: "test-skill".to_string(),
            description: "A test skill".to_string(),
            tools: vec![],
            workflows: vec![],
            license: None,
            system_layer: false,
            tool_filter: None,
            kind: Some("capability".to_string()),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let decoded: SkillMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.name, "test-skill");
    }

    #[test]
    fn test_registry_all_skill_names() {
        let registry = SkillRegistry {
            skills_dir: PathBuf::from("./skills"),
            groups: vec![
                SkillGroup {
                    name: "doc".to_string(),
                    description: "".to_string(),
                },
                SkillGroup {
                    name: "dev".to_string(),
                    description: "".to_string(),
                },
                SkillGroup {
                    name: "xlsx".to_string(),
                    description: "".to_string(),
                },
            ],
        };

        let names = registry.all_skill_names();
        assert!(names.contains(&"doc"));
        assert!(names.contains(&"dev"));
        assert!(names.contains(&"xlsx"));
    }

    #[test]
    fn test_registry_contains() {
        let registry = SkillRegistry {
            skills_dir: PathBuf::from("./skills"),
            groups: vec![SkillGroup {
                name: "skill-a".to_string(),
                description: "".to_string(),
            }],
        };

        assert!(registry.contains("skill-a"));
        assert!(!registry.contains("skill-b"));
    }

    #[test]
    fn test_skill_dirs() {
        let skill = Skill {
            metadata: SkillMetadata {
                name: "test".to_string(),
                description: "desc".to_string(),
                tools: vec!["Calculator".to_string()],
                workflows: vec![],
                license: None,
                system_layer: false,
                tool_filter: None,
                kind: Some("capability".to_string()),
            },
            instructions: "# Hello".to_string(),
            base_path: Some(PathBuf::from("/skills/test")),
        };

        assert_eq!(
            skill.reference_dir(),
            Some(PathBuf::from("/skills/test/reference"))
        );
        assert_eq!(
            skill.scripts_dir(),
            Some(PathBuf::from("/skills/test/scripts"))
        );
    }
}
