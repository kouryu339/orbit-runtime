//! Anthropic Messages API compatible backend.

use reqwest::Client;
use serde_json::{json, Value};

use crate::classify::{classify_network_error, json_response_or_error};
use crate::error::ApiError;
use crate::retry::{retry_with_backoff, RetryPolicy};
use crate::types::{ChatMessage, FunctionCall, LlmResponse, TokenUsage, ToolCall, ToolDefinition};

pub async fn call_inner(
    messages: &[ChatMessage],
    tools: &[ToolDefinition],
    model: &str,
    base_url: &str,
    api_key: &str,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    force_tool_name: Option<&str>,
) -> crate::error::Result<LlmResponse> {
    let client = Client::new();
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));

    let system = build_anthropic_system(messages);
    let msgs = build_anthropic_messages(messages);
    crate::diagnostics::log_provider_messages(
        "anthropic_compat",
        "anthropic_messages",
        model,
        &url,
        false,
        &msgs,
        tools.len(),
    );

    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens.unwrap_or(4096),
        "messages": msgs,
    });

    if let Some(system) = system {
        body["system"] = system;
    }
    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    if let Some(p) = top_p {
        body["top_p"] = json!(p);
    }

    if !tools.is_empty() {
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.function.name,
                    "description": t.function.description,
                    "input_schema": t.function.parameters,
                })
            })
            .collect();
        body["tools"] = json!(tools_json);
    }
    if let Some(name) = force_tool_name {
        body["tool_choice"] = json!({"type": "tool", "name": name});
    }

    tracing::debug!(
        "Anthropic request: model={}, messages={}",
        model,
        msgs.len()
    );

    let body = std::sync::Arc::new(body);
    let url = std::sync::Arc::new(url);
    let key = std::sync::Arc::new(api_key.to_string());
    let model_owned = std::sync::Arc::new(model.to_string());
    let runtime_headers = std::sync::Arc::new(crate::request_context::current_request_headers());
    crate::request_context::validate_header_transport(url.as_ref(), runtime_headers.as_ref())?;

    let resp: Value = retry_with_backoff(RetryPolicy::default(), model, |_attempt| {
        let client = client.clone();
        let body = body.clone();
        let url = url.clone();
        let key = key.clone();
        let model = model_owned.clone();
        let runtime_headers = runtime_headers.clone();
        async move {
            let req = client
                .post(url.as_str())
                .header("x-api-key", key.as_str())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(body.as_ref());
            let req = crate::request_context::apply_request_headers(req, runtime_headers.as_ref());
            let resp = req.send().await.map_err(|e| classify_network_error(&e))?;
            json_response_or_error(resp, model.as_str()).await
        }
    })
    .await?;

    if resp.get("type").and_then(|v| v.as_str()) == Some("error") {
        let msg = resp
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误");
        return Err(ApiError::LlmFailed(format!(
            "API 错误 [{}]: {}",
            model, msg
        )));
    }

    let content_blocks = resp["content"]
        .as_array()
        .ok_or_else(|| ApiError::LlmFailed("响应缺少 content 字段".into()))?;

    let mut text_content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in content_blocks {
        match block["type"].as_str().unwrap_or("") {
            "text" => {
                if let Some(t) = block["text"].as_str() {
                    text_content.push_str(t);
                }
            }
            "tool_use" => {
                let id = block["id"].as_str().unwrap_or("").to_string();
                let name = block["name"].as_str().unwrap_or("").to_string();
                let arguments =
                    serde_json::to_string(&block["input"]).unwrap_or_else(|_| "{}".into());
                if !name.is_empty() {
                    tool_calls.push(ToolCall {
                        id,
                        call_type: Some("function".into()),
                        function: FunctionCall { name, arguments },
                    });
                }
            }
            _ => {}
        }
    }

    if !tool_calls.is_empty() {
        tracing::info!(
            "Anthropic model {} requested {} tools: {}",
            model,
            tool_calls.len(),
            tool_calls
                .iter()
                .map(|tc| tc.function.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let tokens = resp.get("usage").map(|u| TokenUsage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(LlmResponse {
        content: text_content,
        tokens,
        cached_tokens: 0,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        reasoning_content: None,
    })
}

fn build_anthropic_system(messages: &[ChatMessage]) -> Option<Value> {
    let blocks = messages
        .iter()
        .filter(|m| m.role == "system" && !m.content.trim().is_empty())
        .map(|m| {
            let mut block = json!({
                "type": "text",
                "text": m.content,
            });
            if m.cache_control {
                block["cache_control"] = json!({"type": "ephemeral"});
            }
            block
        })
        .collect::<Vec<_>>();
    (!blocks.is_empty()).then(|| json!(blocks))
}

fn build_anthropic_messages(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .filter(|m| m.role != "system")
        .filter_map(|m| {
            if let Some(ref id) = m.tool_call_id {
                if id.trim().is_empty() {
                    return None;
                }
                let content = if m.content.trim().is_empty() {
                    "(empty tool result)"
                } else {
                    m.content.as_str()
                };
                let mut block = json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": content,
                });
                if m.cache_control {
                    block["cache_control"] = json!({"type": "ephemeral"});
                }
                return Some(json!({
                    "role": "user",
                    "content": [block],
                }));
            }

            let mut blocks: Vec<Value> = Vec::new();
            if !m.content.trim().is_empty() {
                let mut block = json!({
                    "type": "text",
                    "text": m.content,
                });
                if m.cache_control {
                    block["cache_control"] = json!({"type": "ephemeral"});
                }
                blocks.push(block);
            }

            if let Some(ref tcs) = m.tool_calls {
                blocks.extend(tcs.iter().filter_map(|tc| {
                    if tc.id.trim().is_empty() || tc.function.name.trim().is_empty() {
                        return None;
                    }
                    Some(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.function.name,
                        "input": serde_json::from_str::<Value>(&tc.function.arguments)
                            .unwrap_or_else(|_| json!({})),
                    }))
                }));
            }

            if blocks.is_empty() {
                return None;
            }

            Some(json!({
                "role": m.role,
                "content": blocks,
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_projection_drops_empty_text_messages() {
        let messages = vec![
            ChatMessage::system(""),
            ChatMessage::assistant("  "),
            ChatMessage::user("hello"),
        ];

        assert!(build_anthropic_system(&messages).is_none());

        let projected = build_anthropic_messages(&messages);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0]["role"], "user");
        assert_eq!(projected[0]["content"][0]["type"], "text");
        assert_eq!(projected[0]["content"][0]["text"], "hello");
    }

    #[test]
    fn anthropic_projection_marks_cached_system_block() {
        let messages = vec![ChatMessage::system_cached("stable instructions")];

        let projected = build_anthropic_system(&messages).unwrap();

        assert_eq!(projected[0]["type"], "text");
        assert_eq!(projected[0]["text"], "stable instructions");
        assert_eq!(projected[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn anthropic_projection_marks_cached_history_block() {
        let mut message = ChatMessage::user("conversation prefix");
        message.cache_control = true;

        let projected = build_anthropic_messages(&[message]);

        assert_eq!(projected[0]["content"][0]["text"], "conversation prefix");
        assert_eq!(
            projected[0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn anthropic_projection_marks_cached_tool_result_block() {
        let mut message = ChatMessage::tool_with_id("result", "call-1", "Tool");
        message.cache_control = true;

        let projected = build_anthropic_messages(&[message]);

        assert_eq!(projected[0]["content"][0]["type"], "tool_result");
        assert_eq!(
            projected[0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn anthropic_projection_never_emits_empty_tool_content_blocks() {
        let mut assistant = ChatMessage::assistant("");
        assistant.tool_calls = Some(vec![]);
        let mut tool = ChatMessage::user("");
        tool.tool_call_id = Some("call-1".to_string());

        let projected = build_anthropic_messages(&[assistant, tool]);

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0]["role"], "user");
        assert_eq!(projected[0]["content"][0]["type"], "tool_result");
        assert_eq!(projected[0]["content"][0]["tool_use_id"], "call-1");
        assert_eq!(projected[0]["content"][0]["content"], "(empty tool result)");
    }
}
