//! Model-scoped reasoning settings, encoded only by the selected API paradigm.
use crate::config::ApiParadigm;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingMode {
    Disabled,
    Enabled,
    Adaptive,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningSummary {
    Auto,
    Concise,
    Detailed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReasoningConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ReasoningSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingMode>,
    #[serde(
        alias = "budgetTokens",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub budget_tokens: Option<u32>,
}

impl ReasoningConfig {
    pub fn validate(&self, paradigm: ApiParadigm) -> Result<(), String> {
        match paradigm {
            ApiParadigm::OpenAiChatCompletions | ApiParadigm::OpenAiResponses => {
                if self.thinking.is_some() || self.budget_tokens.is_some() {
                    return Err("thinking and budget_tokens require anthropic_messages".into());
                }
                if paradigm == ApiParadigm::OpenAiChatCompletions && self.summary.is_some() {
                    return Err("summary requires openai_responses".into());
                }
            }
            ApiParadigm::AnthropicMessages => {
                if self.summary.is_some() {
                    return Err("summary requires openai_responses".into());
                }
                if matches!(
                    self.effort,
                    Some(ReasoningEffort::None | ReasoningEffort::Minimal)
                ) {
                    return Err(
                        "Anthropic effort does not accept none/minimal; use thinking=disabled"
                            .into(),
                    );
                }
                match (self.thinking, self.budget_tokens) {
                    (Some(ThinkingMode::Enabled), Some(budget)) if budget >= 1024 => {}
                    (Some(ThinkingMode::Enabled), _) => {
                        return Err("enabled thinking requires budget_tokens >= 1024".into())
                    }
                    (_, Some(_)) => return Err("budget_tokens requires thinking=enabled".into()),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub(crate) fn apply(&self, paradigm: ApiParadigm, body: &mut Value) -> crate::Result<()> {
        let invalid = |message: String| {
            crate::error::ApiError::Fatal(crate::error::FatalError::config_error(format!(
                "invalid reasoning config: {message}"
            )))
        };
        self.validate(paradigm).map_err(invalid)?;
        let active = match paradigm {
            ApiParadigm::OpenAiChatCompletions => {
                if let Some(effort) = self.effort {
                    body["reasoning_effort"] = json!(effort);
                    if let Some(limit) = body.as_object_mut().unwrap().remove("max_tokens") {
                        body["max_completion_tokens"] = limit;
                    }
                }
                self.effort.is_some_and(|e| e != ReasoningEffort::None)
            }
            ApiParadigm::OpenAiResponses => {
                if let Some(effort) = self.effort {
                    body["reasoning"]["effort"] = json!(effort);
                }
                if let Some(summary) = self.summary {
                    body["reasoning"]["summary"] = json!(summary);
                }
                self.effort.is_some_and(|e| e != ReasoningEffort::None)
            }
            ApiParadigm::AnthropicMessages => {
                if let Some(effort) = self.effort {
                    body["output_config"]["effort"] = json!(effort);
                }
                if let Some(mode) = self.thinking {
                    body["thinking"]["type"] = json!(mode);
                }
                if let Some(budget) = self.budget_tokens {
                    if body["max_tokens"].as_u64().unwrap_or(0) <= u64::from(budget) {
                        return Err(invalid(
                            "budget_tokens must be less than request max_tokens".into(),
                        ));
                    }
                    body["thinking"]["budget_tokens"] = json!(budget);
                }
                let enabled = matches!(
                    self.thinking,
                    Some(ThinkingMode::Enabled | ThinkingMode::Adaptive)
                );
                if enabled
                    && body
                        .get("tool_choice")
                        .is_some_and(|v| matches!(v["type"].as_str(), Some("tool" | "any")))
                {
                    return Err(invalid(
                        "Anthropic thinking requires automatic tool choice".into(),
                    ));
                }
                enabled
            }
        };
        // Standard reasoning APIs may reject sampling parameters. Do not guess
        // provider-specific exceptions or silently retry without reasoning.
        if active {
            body.as_object_mut().unwrap().remove("temperature");
            body.as_object_mut().unwrap().remove("top_p");
        }
        tracing::debug!(?paradigm, ?self, "applied model reasoning configuration");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encodes_only_selected_paradigm_and_omits_sampling() {
        for paradigm in [
            ApiParadigm::OpenAiChatCompletions,
            ApiParadigm::OpenAiResponses,
            ApiParadigm::AnthropicMessages,
        ] {
            let config = ReasoningConfig {
                effort: Some(ReasoningEffort::High),
                thinking: (paradigm == ApiParadigm::AnthropicMessages)
                    .then_some(ThinkingMode::Adaptive),
                ..Default::default()
            };
            let mut body = json!({"temperature":0.5,"top_p":0.8,"max_tokens":4096});
            config.apply(paradigm, &mut body).unwrap();
            assert!(body.get("temperature").is_none() && body.get("top_p").is_none());
            match paradigm {
                ApiParadigm::OpenAiChatCompletions => assert_eq!(body["reasoning_effort"], "high"),
                ApiParadigm::OpenAiResponses => assert_eq!(body["reasoning"]["effort"], "high"),
                ApiParadigm::AnthropicMessages => {
                    assert_eq!(body["output_config"]["effort"], "high");
                    assert_eq!(body["thinking"]["type"], "adaptive");
                }
            }
        }
    }
    #[test]
    fn defaults_leave_request_unchanged() {
        let mut body = json!({"temperature":0.5,"max_tokens":4096});
        let before = body.clone();
        ReasoningConfig::default()
            .apply(ApiParadigm::OpenAiChatCompletions, &mut body)
            .unwrap();
        assert_eq!(body, before);
    }
    #[test]
    fn rejects_wrong_paradigm_and_conflicting_budget() {
        let config = ReasoningConfig {
            thinking: Some(ThinkingMode::Enabled),
            budget_tokens: Some(2048),
            ..Default::default()
        };
        assert!(config.validate(ApiParadigm::OpenAiResponses).is_err());
        assert!(config
            .apply(
                ApiParadigm::AnthropicMessages,
                &mut json!({"max_tokens":2048})
            )
            .is_err());
        assert!(config
            .apply(
                ApiParadigm::AnthropicMessages,
                &mut json!({"max_tokens":4096,"tool_choice":{"type":"any"}})
            )
            .is_err());
        let mut body = json!({"max_tokens":4096});
        config
            .apply(ApiParadigm::AnthropicMessages, &mut body)
            .unwrap();
        assert_eq!(
            body["thinking"],
            json!({"type":"enabled","budget_tokens":2048})
        );
        assert!(ReasoningConfig {
            summary: Some(ReasoningSummary::Auto),
            ..Default::default()
        }
        .validate(ApiParadigm::OpenAiChatCompletions)
        .is_err());
        assert!(serde_json::from_value::<ReasoningConfig>(json!({"effort":"typo"})).is_err());
    }
}
