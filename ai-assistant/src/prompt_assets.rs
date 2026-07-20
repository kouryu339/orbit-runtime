use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone)]
struct PromptAssetConfig {
    language: String,
}

impl Default for PromptAssetConfig {
    fn default() -> Self {
        Self {
            language: "zh".to_string(),
        }
    }
}

static CONFIG: OnceLock<RwLock<PromptAssetConfig>> = OnceLock::new();

fn config() -> &'static RwLock<PromptAssetConfig> {
    CONFIG.get_or_init(|| RwLock::new(PromptAssetConfig::default()))
}

pub fn default_prompts_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AI_ASSISTANT_PROMPTS_DIR") {
        let path = PathBuf::from(dir);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts")
}

pub fn set_prompts_dir(path: impl Into<PathBuf>) {
    let _ = path.into();
}

pub fn prompts_dir() -> PathBuf {
    default_prompts_dir()
}

pub fn set_language(language: &str) -> Result<String, String> {
    let normalized = normalize_language(language)?;
    if let Ok(mut guard) = config().write() {
        guard.language = normalized.clone();
    }
    Ok(normalized)
}

pub fn language() -> String {
    config()
        .read()
        .map(|guard| guard.language.clone())
        .unwrap_or_else(|_| "zh".to_string())
}

pub fn normalize_language(language: &str) -> Result<String, String> {
    let value = language.trim().to_ascii_lowercase().replace('_', "-");
    let normalized = match value.as_str() {
        "" => "zh",
        "zh" | "zh-cn" | "zh-hans" | "cn" | "chinese" => "zh",
        "en" | "en-us" | "en-gb" | "english" => "en",
        other => {
            return Err(format!(
                "unsupported language '{other}', expected 'zh' or 'en'"
            ))
        }
    };
    Ok(normalized.to_string())
}

pub fn template(name: &str) -> String {
    let current_language = language();
    embedded_template(&current_language, name)
        .or_else(|| embedded_template("en", name))
        .or_else(|| embedded_template("zh", name))
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            tracing::warn!("embedded prompt template '{}' not found", name);
            String::new()
        })
}

pub fn render(name: &str, replacements: &[(&str, &str)]) -> String {
    let mut rendered = template(name);
    for (key, value) in replacements {
        rendered = rendered.replace(key, value);
    }
    rendered.trim().to_string()
}

include!(concat!(env!("OUT_DIR"), "/embedded_prompts.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_templates_are_embedded() {
        assert!(
            embedded_template("zh", "retry_invalid_response.md")
                .unwrap()
                .trim()
                .len()
                > 0
        );
        assert!(
            embedded_template("zh", "default_persona.md")
                .unwrap()
                .trim()
                .len()
                > 0
        );
    }
}
