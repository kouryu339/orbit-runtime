//! system 层 skill 是框架行为规范，不随用户目录变化，
//! 新增 system skill 时：
//! 1. 在 `skills/system/` 下创建目录 + `SKILL.md`
//! 2. 在本文件增加一条 `include_str!` + `EMBEDDED_SYSTEM_SKILLS` 数组条目

use super::types::{Skill, SkillEntry, SkillLoadState, SkillMetadata};

fn parse_embedded(content: &'static str) -> Skill {
    // 分离 YAML frontmatter 和 body
    let content = content.trim_start_matches('\u{feff}'); // 去 BOM
    let after_open = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
        .expect("内嵌 SKILL.md 缺少 frontmatter 开头 '---'");

    let (frontmatter, body) = if let Some(pos) = after_open.find("\n---\n") {
        (&after_open[..pos], &after_open[pos + 5..])
    } else if let Some(pos) = after_open.find("\n---\r\n") {
        (&after_open[..pos], &after_open[pos + 6..])
    } else {
        panic!("内嵌 SKILL.md 缺少 frontmatter 结尾 '---'");
    };

    let meta: SkillMetadata = serde_yaml::from_str(frontmatter)
        .unwrap_or_else(|e| panic!("内嵌 SKILL.md frontmatter 解析失败: {}", e));

    Skill {
        metadata: meta,
        instructions: body.trim().to_string(),
        base_path: None, // 内嵌 skill 无文件路径
    }
}

static EMBEDDED_SYSTEM_SKILL_SOURCES: &[&str] = &[
    include_str!("../../skills/system/thinking/SKILL.md"),
    include_str!("../../skills/system/thinking-pro/SKILL.md"),
    include_str!("../../skills/system/workflow_editor/SKILL.md"),
    include_str!("../../skills/system/agent_test_supervisor/SKILL.md"),
    include_str!("../../skills/system/agent_test_adversary/SKILL.md"),
];

/// 构建所有内嵌 system skill 的 SkillEntry 列表（在 SkillManager::new_with_embedded 中调用）
pub fn embedded_system_entries() -> Vec<(String, SkillEntry)> {
    let mut entries = EMBEDDED_SYSTEM_SKILL_SOURCES
        .iter()
        .map(|src| {
            let skill = parse_embedded(src);
            let name = skill.metadata.name.clone();
            let entry = SkillEntry {
                name: name.clone(),
                state: SkillLoadState::FullyLoaded,
                skill: Some(skill),
            };
            (name, entry)
        })
        .collect::<Vec<_>>();
    for (source, alias) in [
        ("thinking", "thinking-fc"),
        ("thinking-pro", "thinking-pro-fc"),
    ] {
        if let Some((_, entry)) = entries.iter().find(|(name, _)| name == source) {
            let mut alias_entry = entry.clone();
            alias_entry.name = alias.to_string();
            if let Some(skill) = alias_entry.skill.as_mut() {
                skill.metadata.name = alias.to_string();
                skill.metadata.description = format!(
                    "{} Native function-calling transport profile.",
                    skill.metadata.description
                );
            }
            entries.push((alias.to_string(), alias_entry));
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_agent_test_roles() {
        let entries = embedded_system_entries();
        let adversary = entries
            .iter()
            .find(|(name, _)| name == "agent_test_adversary")
            .and_then(|(_, entry)| entry.skill.as_ref())
            .expect("agent test adversary role should be embedded");
        let supervisor = entries
            .iter()
            .find(|(name, _)| name == "agent_test_supervisor")
            .and_then(|(_, entry)| entry.skill.as_ref())
            .expect("agent test supervisor role should be embedded");

        assert!(adversary.metadata.is_role());
        assert_eq!(adversary.metadata.tools, vec!["AdversaryConclude"]);
        assert!(supervisor.metadata.is_role());
    }

    #[test]
    fn thinking_pro_does_not_grant_legacy_script_tools() {
        let entries = embedded_system_entries();
        let skill = entries
            .iter()
            .find(|(name, _)| name == "thinking-pro")
            .and_then(|(_, entry)| entry.skill.as_ref())
            .unwrap();
        assert!(skill.metadata.tools.contains(&"PlanWrite".to_string()));
        for name in [
            "executeWorkflowScript",
            "createWorkflowDraft",
            "updateWorkflow",
        ] {
            assert!(!skill.metadata.tools.contains(&name.to_string()));
        }
        assert!(!skill.instructions.contains("N: EXEC"));
    }

    #[test]
    fn thinking_skills_define_behavior_without_competing_with_role() {
        let entries = embedded_system_entries();

        for name in ["thinking", "thinking-pro"] {
            let skill = entries
                .iter()
                .find(|(entry_name, _)| entry_name == name)
                .and_then(|(_, entry)| entry.skill.as_ref())
                .unwrap_or_else(|| panic!("{name} should be embedded"));

            for tool in ["GetSkillsList", "UpdateSkills", "Grep", "GrepJson"] {
                assert!(
                    !skill.metadata.tools.iter().any(|name| name == tool),
                    "{name} must not activate host-selected tool {tool}"
                );
            }
            assert!(skill.metadata.system_layer);
            assert!(!skill.metadata.is_role());
            assert!(!skill.instructions.trim_start().starts_with("你是"));
            assert!(skill
                .instructions
                .contains("以下面的思考执行模型为基础行为指导"));
            assert!(!skill.instructions.contains("本 Skill"));
            assert!(skill.instructions.contains("不与角色定义竞争"));
            assert!(skill.instructions.contains("调用工具前"));
            assert!(skill.instructions.contains("即将执行什么以及为什么"));
            assert!(skill.instructions.contains("无法收敛为可靠行动"));
            assert!(skill.instructions.contains("用户能理解的具体话语"));
            assert!(skill.instructions.contains("立即停止追问并开始行动"));
            assert!(skill.instructions.contains("PlanWrite"));
            assert!(skill.instructions.contains("用户明确改变或取消目标"));
            assert!(skill.instructions.contains("当前高优先级任务"));
        }
    }

    #[test]
    fn workflow_editor_does_not_grant_legacy_script_tools() {
        let entries = embedded_system_entries();
        let skill = entries
            .iter()
            .find(|(name, _)| name == "workflow_editor")
            .and_then(|(_, entry)| entry.skill.as_ref())
            .unwrap();
        assert!(skill.metadata.tools.contains(&"listWorkflows".to_string()));
        for name in [
            "executeWorkflowScript",
            "createWorkflowDraft",
            "updateWorkflow",
        ] {
            assert!(!skill.metadata.tools.contains(&name.to_string()));
        }
    }
}
