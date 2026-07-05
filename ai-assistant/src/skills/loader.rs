//! - 解析 SKILL.md（YAML frontmatter + Markdown body）
//! - 加载 skills.json 注册表配置
//! - 按目录约定扫描 Skill 集

use crate::skills::types::{Skill, SkillMetadata, SkillRegistry};
use corework::error::{FrameworkError, Result};
use std::path::Path;

// ============================================================================
// SKILL.md 解析
// ============================================================================

/// 从文件路径加载完整 Skill（Tier 1 + Tier 2）
pub async fn load_skill_file(path: &Path) -> Result<Skill> {
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        FrameworkError::InvalidData(format!("读取 Skill 文件失败 {:?}: {}", path, e))
    })?;

    let mut skill = parse_skill_content(&content)?;

    skill.base_path = path.parent().map(|p| p.to_path_buf());

    Ok(skill)
}

/// 仅加载元数据（Tier 1）——不读取 instructions body
/// 适用于首次扫描时快速获取所有 Skill 的 name + description
pub async fn load_skill_metadata(path: &Path) -> Result<SkillMetadata> {
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        FrameworkError::InvalidData(format!("读取 Skill 文件失败 {:?}: {}", path, e))
    })?;

    parse_frontmatter(&content)
}

/// 解析完整 SKILL.md 内容
/// ## 格式
/// ```markdown
/// ---
/// name: skill-identifier
/// description: "Trigger description..."
/// tools: ["ToolA", "ToolB"]
/// workflows: ["工作流名称A", "工作流名称B"]   # 可选：声明后激活该 skill 时只注入这些工作流
/// license: Optional
/// ---
/// # Instructions body (Markdown)
/// ...
/// ```
/// ## workflows 字段说明
/// - 声明后：激活该 skill 时，`已注册工作流` 章节只注入声明的工作流（精确注入）
/// - 不声明：激活该 skill 时注入全部注册的工作流（适用于 workflow skill 等通用场景）
pub fn parse_skill_content(content: &str) -> Result<Skill> {
    let (frontmatter_str, body) = split_frontmatter(content)?;
    let metadata = parse_yaml_frontmatter(frontmatter_str)?;

    Ok(Skill {
        metadata,
        instructions: body.to_string(),
        base_path: None,
    })
}

/// 仅解析 frontmatter 部分
fn parse_frontmatter(content: &str) -> Result<SkillMetadata> {
    let (frontmatter_str, _) = split_frontmatter(content)?;
    parse_yaml_frontmatter(frontmatter_str)
}

/// 分离 YAML frontmatter 和 Markdown body
/// 返回 `(yaml_str, body_str)`
fn split_frontmatter(content: &str) -> Result<(&str, &str)> {
    // 去除 UTF-8 BOM（如果存在）
    let content = content.strip_prefix('\u{FEFF}').unwrap_or(content);

    // 支持 "---\n" 和 "---\r\n" 开头
    let trimmed = content.trim_start();

    if !trimmed.starts_with("---") {
        return Err(FrameworkError::InvalidData(
            "SKILL.md 必须以 YAML frontmatter (---) 开头".to_string(),
        ));
    }

    // 跳过首个 "---" 行
    let after_first = match trimmed.strip_prefix("---") {
        Some(rest) => rest.trim_start_matches(['\r', '\n']),
        None => {
            return Err(FrameworkError::InvalidData(
                "无法解析 SKILL.md frontmatter".to_string(),
            ))
        }
    };

    // 查找结束标记 "---"
    // 支持 \n---\n 和 \n---\r\n
    let end_markers = ["\n---\n", "\n---\r\n"];
    let mut split_pos = None;
    let mut marker_len = 0;

    for marker in &end_markers {
        if let Some(pos) = after_first.find(marker) {
            split_pos = Some(pos);
            marker_len = marker.len();
            break;
        }
    }

    if split_pos.is_none() && after_first.ends_with("\n---") {
        split_pos = Some(after_first.len() - 4);
        marker_len = 4;
    }

    match split_pos {
        Some(pos) => {
            let yaml_str = &after_first[..pos];
            let body = &after_first[pos + marker_len..];
            Ok((yaml_str.trim(), body.trim()))
        }
        None => Err(FrameworkError::InvalidData(
            "SKILL.md 缺少 frontmatter 结束标记 (---)".to_string(),
        )),
    }
}

/// 解析 YAML frontmatter 为 SkillMetadata
fn parse_yaml_frontmatter(yaml_str: &str) -> Result<SkillMetadata> {
    let metadata: SkillMetadata = serde_yaml::from_str(yaml_str)
        .map_err(|e| FrameworkError::InvalidData(format!("YAML 解析错误: {}", e)))?;

    // 校验必填字段
    if metadata.name.is_empty() {
        return Err(FrameworkError::InvalidData(
            "Skill 必须有非空 'name' 字段".to_string(),
        ));
    }
    if metadata.description.is_empty() {
        return Err(FrameworkError::InvalidData(
            "Skill 必须有非空 'description' 字段".to_string(),
        ));
    }

    Ok(metadata)
}

// ============================================================================
// 注册表加载
// ============================================================================

/// 从 JSON 文件加载 SkillRegistry
pub async fn load_registry(path: &Path) -> Result<SkillRegistry> {
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        FrameworkError::InvalidData(format!("读取注册表文件失败 {:?}: {}", path, e))
    })?;

    let registry: SkillRegistry = serde_json::from_str(&content)
        .map_err(|e| FrameworkError::InvalidData(format!("注册表 JSON 解析错误: {}", e)))?;

    Ok(registry)
}

// ============================================================================
// 目录扫描
// ============================================================================

/// 扫描目录发现所有 Skill（仅加载元数据）
/// ## 约定目录结构
/// ```text
/// skills_dir/
///   skill-a/
///     SKILL.md
///     reference/
///     scripts/
///   skill-b/
///     SKILL.md
/// ```
/// 返回 `Vec<(name, SkillMetadata)>`，name 取自目录名
pub async fn discover_skills(skills_dir: &Path) -> Result<Vec<(String, SkillMetadata)>> {
    let mut result = Vec::new();

    if !skills_dir.exists() {
        tracing::warn!("Skills 目录不存在: {:?}", skills_dir);
        return Ok(result);
    }

    let mut entries = tokio::fs::read_dir(skills_dir).await.map_err(|e| {
        FrameworkError::InvalidData(format!("读取 Skills 目录失败 {:?}: {}", skills_dir, e))
    })?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| FrameworkError::InvalidData(format!("读取目录条目失败: {}", e)))?
    {
        let path = entry.path();
        if !path.is_dir() {
            tracing::debug!("跳过非目录: {:?}", path);
            continue;
        }

        let skill_file = path.join("SKILL.md");
        if !skill_file.exists() {
            tracing::debug!("跳过没有 SKILL.md 的目录: {:?}", path);
            continue;
        }

        let dir_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        tracing::debug!("尝试加载 skill: {}", dir_name);
        match load_skill_metadata(&skill_file).await {
            Ok(meta) => {
                tracing::debug!("成功加载 skill '{}' 的元数据", dir_name);
                result.push((dir_name, meta));
            }
            Err(e) => {
                // 读取文件内容用于调试
                if let Ok(content) = tokio::fs::read_to_string(&skill_file).await {
                    let preview = content.chars().take(100).collect::<String>();
                    tracing::warn!(
                        "跳过无效 Skill '{}': {}，文件前100字符: {:?}",
                        dir_name,
                        e,
                        preview
                    );
                } else {
                    tracing::warn!("跳过无效 Skill '{}': {}", dir_name, e);
                }
            }
        }
    }

    // 按名称排序以保证稳定顺序
    result.sort_by(|a, b| a.0.cmp(&b.0));

    tracing::debug!(
        "discover_skills 完成，共发现 {} 个 skills: {:?}",
        result.len(),
        result.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    Ok(result)
}

/// 按名称加载单个 Skill 的完整内容
/// `skills_dir`: Skills 根目录
/// `name`: Skill 名称（对应子目录名）
pub async fn load_skill_by_name(skills_dir: &Path, name: &str) -> Result<Skill> {
    let skill_file = skills_dir.join(name).join("SKILL.md");

    if !skill_file.exists() {
        return Err(FrameworkError::InvalidData(format!(
            "Skill '{}' 不存在: {:?}",
            name, skill_file
        )));
    }

    load_skill_file(&skill_file).await
}

// ============================================================================
// Tier 3: Reference 文件加载
// ============================================================================

/// 列出 Skill 的所有 reference 文件
pub async fn list_references(skill: &Skill) -> Result<Vec<String>> {
    let ref_dir = match skill.reference_dir() {
        Some(dir) if dir.exists() => dir,
        _ => return Ok(Vec::new()),
    };

    let mut files = Vec::new();
    let mut entries = tokio::fs::read_dir(&ref_dir)
        .await
        .map_err(|e| FrameworkError::InvalidData(format!("读取 reference 目录失败: {}", e)))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| FrameworkError::InvalidData(format!("读取 reference 条目失败: {}", e)))?
    {
        if let Some(name) = entry.file_name().to_str() {
            files.push(name.to_string());
        }
    }

    files.sort();
    Ok(files)
}

/// 加载指定 reference 文件内容
pub async fn load_reference(skill: &Skill, filename: &str) -> Result<String> {
    let ref_dir = skill.reference_dir().ok_or_else(|| {
        FrameworkError::InvalidData(format!("Skill '{}' 没有 base_path", skill.name()))
    })?;

    let path = ref_dir.join(filename);
    tokio::fs::read_to_string(&path).await.map_err(|e| {
        FrameworkError::InvalidData(format!("读取 reference 文件 {:?} 失败: {}", path, e))
    })
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_skill_content_standard() {
        let content = "---\nname: test-skill\ndescription: A test skill\n---\n\n# Instructions\n\nDo something.";
        let skill = parse_skill_content(content).unwrap();
        assert_eq!(skill.metadata.name, "test-skill");
        assert_eq!(skill.metadata.description, "A test skill");
        assert!(skill.instructions.contains("# Instructions"));
    }

    #[test]
    fn test_parse_skill_content_with_license() {
        let content = "---\nname: my-skill\ndescription: \"Use when something happens\"\nlicense: MIT\n---\n\nBody here.";
        let skill = parse_skill_content(content).unwrap();
        assert_eq!(skill.metadata.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn test_parse_skill_content_missing_frontmatter() {
        let content = "# No frontmatter\nJust markdown.";
        assert!(parse_skill_content(content).is_err());
    }

    #[test]
    fn test_parse_skill_content_missing_name() {
        let content = "---\ndescription: A test\n---\nBody.";
        assert!(parse_skill_content(content).is_err());
    }

    #[test]
    fn test_parse_skill_content_empty_description() {
        let content = "---\nname: bad\ndescription: \"\"\n---\nBody.";
        assert!(parse_skill_content(content).is_err());
    }

    #[test]
    fn test_parse_mcp_format() {
        // 真实 MCP Skill 的 frontmatter 格式
        let content = r#"---
name: xlsx
description: "Use this skill any time a spreadsheet file is the primary input or output."
license: Proprietary. LICENSE.txt has complete terms
---

# Requirements for Outputs

## All Excel files

Use a consistent font."#;

        let skill = parse_skill_content(content).unwrap();
        assert_eq!(skill.metadata.name, "xlsx");
        assert!(skill.metadata.description.contains("spreadsheet"));
        assert!(skill.instructions.contains("Requirements for Outputs"));
    }
}
