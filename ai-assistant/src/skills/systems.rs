//! 通过 `#[buns_system]` 宏自动注册，状态机中通过名称动态调用。
//! ## 初始化
//! 应用启动时调用 `init_skill_manager(manager)` 设置全局共享实例。
//! |--------------------|------------------------------ |
//! | `DiscoverSkills`   | 扫描目录发现可用 Skills         |
//! | `SkillCatalog`     | 获取技能目录提示词（Tier 1）     |
//! | `SkillPrompt`      | 获取已激活 Skill 提示词         |
//! | `LoadReference`    | 加载 Tier 3 reference 文件     |

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use corework::buns_system;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;

use super::{SkillManager, SkillMetadata};

// ============================================================================
// 全局共享 SkillManager
// ============================================================================

pub static SKILL_MANAGER: OnceLock<Arc<RwLock<SkillManager>>> = OnceLock::new();

/// 初始化全局 SkillManager
pub fn init_skill_manager(manager: SkillManager) {
    SKILL_MANAGER.set(Arc::new(RwLock::new(manager))).ok();
}

pub async fn replace_skill_manager(manager: SkillManager) {
    if let Some(existing) = SKILL_MANAGER.get() {
        *existing.write().await = manager;
        return;
    }
    SKILL_MANAGER.set(Arc::new(RwLock::new(manager))).ok();
}

pub(crate) fn mgr() -> &'static Arc<RwLock<SkillManager>> {
    SKILL_MANAGER
        .get()
        .expect("SkillManager 未初始化，请先调用 init_skill_manager()")
}

pub mod system_names {
    pub const DISCOVER_SKILLS: &str = "DiscoverSkills";
    pub const SKILL_CATALOG: &str = "SkillCatalog";
    pub const SKILL_PROMPT: &str = "SkillPrompt";
    pub const LOAD_REFERENCE: &str = "LoadReference";
}

// ============================================================================
// Input / Output 模型
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverSkillsInput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverSkillsOutput {
    pub skills: Vec<SkillMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCatalogInput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCatalogOutput {
    pub catalog: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPromptInput {
    pub names: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPromptOutput {
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadReferenceInput {
    pub skill_name: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadReferenceOutput {
    pub content: String,
}

// ============================================================================
// 1. DiscoverSkills
// ============================================================================

#[buns_system(
    "DiscoverSkills",
    description = "扫描目录发现可用 Skills（Tier 1 元数据）"
)]
pub struct DiscoverSkillsSystem;

#[async_trait]
impl SystemOperation for DiscoverSkillsSystem {
    type Input = DiscoverSkillsInput;
    type Output = DiscoverSkillsOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        _input: DiscoverSkillsInput,
        _ctx: &Context,
    ) -> Result<DiscoverSkillsOutput, FrameworkError> {
        let mut m = mgr().write().await;
        m.discover().await?;
        let skills = m.feature_metadata().into_iter().cloned().collect();
        Ok(DiscoverSkillsOutput { skills })
    }

    fn name(&self) -> &str {
        system_names::DISCOVER_SKILLS
    }

    fn is_idempotent(&self) -> bool {
        true
    }
}

// ============================================================================
// 2. SkillCatalog
// ============================================================================

#[buns_system("SkillCatalog", description = "获取技能目录提示词（Tier 1）")]
pub struct SkillCatalogSystem;

#[async_trait]
impl SystemOperation for SkillCatalogSystem {
    type Input = SkillCatalogInput;
    type Output = SkillCatalogOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        _input: SkillCatalogInput,
        _ctx: &Context,
    ) -> Result<SkillCatalogOutput, FrameworkError> {
        let m = mgr().read().await;
        Ok(SkillCatalogOutput {
            catalog: m.catalog_prompt(),
        })
    }

    fn name(&self) -> &str {
        system_names::SKILL_CATALOG
    }

    fn is_idempotent(&self) -> bool {
        true
    }
}

// ============================================================================
// 4. SkillPrompt
// ============================================================================

#[buns_system("SkillPrompt", description = "获取已激活 Skill 的指导提示词（Tier 2）")]
pub struct SkillPromptSystem;

#[async_trait]
impl SystemOperation for SkillPromptSystem {
    type Input = SkillPromptInput;
    type Output = SkillPromptOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: SkillPromptInput,
        _ctx: &Context,
    ) -> Result<SkillPromptOutput, FrameworkError> {
        let m = mgr().read().await;
        let names: Vec<&str> = input
            .names
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(SkillPromptOutput {
            prompt: m.active_skills_prompt(&names),
        })
    }

    fn name(&self) -> &str {
        system_names::SKILL_PROMPT
    }

    fn is_idempotent(&self) -> bool {
        true
    }
}

// ============================================================================
// 5. LoadReference
// ============================================================================

#[buns_system(
    "LoadReference",
    description = "加载 Skill 的 Reference 文件（Tier 3）"
)]
pub struct LoadReferenceSystem;

#[async_trait]
impl SystemOperation for LoadReferenceSystem {
    type Input = LoadReferenceInput;
    type Output = LoadReferenceOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: LoadReferenceInput,
        _ctx: &Context,
    ) -> Result<LoadReferenceOutput, FrameworkError> {
        let m = mgr().read().await;
        let content = m.load_reference(&input.skill_name, &input.filename).await?;
        Ok(LoadReferenceOutput { content })
    }

    fn name(&self) -> &str {
        system_names::LOAD_REFERENCE
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_names() {
        assert_eq!(system_names::DISCOVER_SKILLS, "DiscoverSkills");
        assert_eq!(system_names::SKILL_CATALOG, "SkillCatalog");
        assert_eq!(system_names::SKILL_PROMPT, "SkillPrompt");
        assert_eq!(system_names::LOAD_REFERENCE, "LoadReference");
    }
}
