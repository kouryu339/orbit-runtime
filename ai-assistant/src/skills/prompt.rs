//! Skill 提示词构建
//! 将已加载的 Skills 转换为 LLM 可用的提示词文本
//! ## 设计
//! 遵循 MCP 三层渐进加载：
//! - Tier 1: 输出所有 Skill 的 name + description（索引/目录）
//! - Tier 2: 输出已触发 Skill 的 instructions（详细指导）
//! - Tier 3: 按需附加 reference 内容

use crate::skills::types::{Skill, SkillMetadata};

/// 构建 Tier 1 技能目录（所有 Skill 的 name + description）
/// ## 输出示例
/// ```text
/// ## 可用技能
/// - **xlsx**: Use this skill any time a spreadsheet file is the primary input or output...
/// - **mcp-builder**: Guide for creating high-quality MCP servers...
/// ```
pub fn build_skills_catalog(skills: &[SkillMetadata]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut output = crate::prompt_assets::template("skills_catalog_header.md")
        .trim()
        .to_string();
    output.push_str("\n\n");
    output.push_str(crate::prompt_assets::template("skills_catalog_preamble.md").trim());
    output.push_str("\n\n");

    for meta in skills {
        output.push_str(&format!("- **{}**: {}\n", meta.name, meta.description));
    }

    output
}

/// 构建 Tier 2 技能指导（单个已触发 Skill 的完整 instructions）
/// 当 AI 判断需要启用某 Skill 后，将其 instructions 追加到 prompt  
/// ## 输出示例
/// ```text
/// ## 技能: xlsx
/// > Use this skill any time a spreadsheet file is the primary input or output.
/// ### 指导内容
/// # Requirements for Outputs
/// ...
/// ```
pub fn build_skill_prompt(skill: &Skill) -> String {
    let mut output =
        crate::prompt_assets::render("skill_prompt_header.md", &[("{{NAME}}", skill.name())]);
    output.push_str("\n\n");
    output.push_str(&format!("> {}\n\n", skill.description()));
    output.push_str(crate::prompt_assets::template("skill_instructions_header.md").trim());
    output.push_str("\n\n");
    output.push_str(&skill.instructions);
    output.push('\n');
    output
}

/// 构建 Tier 3 参考文档附加内容
/// 将外部 reference 文件内容附加到 prompt
pub fn build_reference_prompt(skill_name: &str, filename: &str, content: &str) -> String {
    let mut output = crate::prompt_assets::render(
        "skill_reference_header.md",
        &[("{{FILENAME}}", filename), ("{{SKILL_NAME}}", skill_name)],
    );
    output.push_str("\n\n");
    output.push_str(content);
    output.push('\n');
    output
}

/// 构建多个已触发 Skills 的合并 prompt
pub fn build_active_skills_prompt(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut output = crate::prompt_assets::template("active_skills_header.md")
        .trim()
        .to_string();
    output.push_str("\n\n");
    for skill in skills {
        output.push_str(&build_skill_prompt(skill));
        output.push('\n');
    }
    output
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_skills_catalog_empty() {
        assert_eq!(build_skills_catalog(&[]), "");
    }

    #[test]
    fn test_build_skills_catalog() {
        let metas = vec![
            SkillMetadata {
                name: "xlsx".to_string(),
                description: "Process spreadsheets".to_string(),
                license: None,
                tools: vec![],
                workflows: vec![],
                system_layer: false,
                tool_filter: None,
                kind: Some("capability".to_string()),
            },
            SkillMetadata {
                name: "docx".to_string(),
                description: "Process Word documents".to_string(),
                license: None,
                tools: vec![],
                workflows: vec![],
                system_layer: false,
                tool_filter: None,
                kind: Some("capability".to_string()),
            },
        ];

        let result = build_skills_catalog(&metas);
        assert!(result.contains("## 可用技能"));
        assert!(result.contains("**xlsx**: Process spreadsheets"));
        assert!(result.contains("**docx**: Process Word documents"));
    }

    #[test]
    fn test_build_skill_prompt() {
        let skill = Skill {
            metadata: SkillMetadata {
                name: "test".to_string(),
                description: "A test skill".to_string(),
                license: None,
                tools: vec![],
                workflows: vec![],
                system_layer: false,
                tool_filter: None,
                kind: Some("capability".to_string()),
            },
            instructions: "# Step 1\nDo something.".to_string(),
            base_path: None,
        };

        let result = build_skill_prompt(&skill);
        assert!(result.contains("## 技能: test"));
        assert!(result.contains("> A test skill"));
        assert!(result.contains("# Step 1"));
    }
}
