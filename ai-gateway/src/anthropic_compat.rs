//! Anthropic Messages API compatible backend.

use reqwest::Client;
use serde_json::{json, Value};

use crate::classify::{classify_http_error, classify_network_error, json_response_or_error};
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
    let request_started = std::time::Instant::now();
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

    crate::diagnostics::append_line(format!(
        "[ai-gateway response] {}",
        json!({
            "phase": "completed",
            "stream": false,
            "provider_api": "anthropic_messages",
            "model": model,
            "elapsed_ms": request_started.elapsed().as_millis() as u64,
            "content_chars": text_content.chars().count(),
            "tool_call_count": tool_calls.len(),
        })
    ));

    Ok(LlmResponse {
        content: text_content,
        finish_reason: resp["stop_reason"].as_str().map(str::to_string),
        tokens,
        cached_tokens: 0,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        reasoning_content: None,
        provider_items: None,
    })
}

pub async fn call_inner_streaming<F>(
    messages: &[ChatMessage],
    tools: &[ToolDefinition],
    model: &str,
    base_url: &str,
    api_key: &str,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    force_tool_name: Option<&str>,
    mut on_chunk: F,
) -> crate::error::Result<LlmResponse>
where
    F: FnMut(String) + Send,
{
    use futures_util::StreamExt;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct ToolBlock {
        id: String,
        name: String,
        arguments: String,
    }

    let client = Client::new();
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    let system = build_anthropic_system(messages);
    let msgs = build_anthropic_messages(messages);
    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens.unwrap_or(4096),
        "messages": msgs,
        "stream": true,
    });
    if let Some(system) = system {
        body["system"] = system;
    }
    if let Some(value) = temperature {
        body["temperature"] = json!(value);
    }
    if let Some(value) = top_p {
        body["top_p"] = json!(value);
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools
            .iter()
            .map(|tool| json!({
                "name": tool.function.name,
                "description": tool.function.description,
                "input_schema": tool.function.parameters,
            }))
            .collect::<Vec<_>>());
    }
    if let Some(name) = force_tool_name {
        body["tool_choice"] = json!({"type": "tool", "name": name});
    }
    let body = std::sync::Arc::new(body);
    let url = std::sync::Arc::new(url);
    let key = std::sync::Arc::new(api_key.to_string());
    let model_owned = std::sync::Arc::new(model.to_string());
    let runtime_headers = std::sync::Arc::new(crate::request_context::current_request_headers());
    crate::request_context::validate_header_transport(url.as_ref(), runtime_headers.as_ref())?;
    let response = retry_with_backoff(RetryPolicy::default(), model, |attempt| {
        let client = client.clone();
        let body = body.clone();
        let url = url.clone();
        let key = key.clone();
        let model = model_owned.clone();
        let runtime_headers = runtime_headers.clone();
        async move {
            let request = client
                .post(url.as_str())
                .header("x-api-key", key.as_str())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(body.as_ref());
            let request = crate::request_context::apply_request_headers(request, &runtime_headers);
            let response = request
                .send()
                .await
                .map_err(|error| classify_network_error(&error))?;
            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }
            let headers = response.headers().clone();
            let text = response.text().await.unwrap_or_default();
            tracing::warn!(
                attempt = attempt + 1,
                status = status.as_u16(),
                model = %model,
                error_summary = %text.chars().take(200).collect::<String>(),
                "Anthropic streaming request failed"
            );
            Err(classify_http_error(status, &text, &headers, &model))
        }
    })
    .await?;

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut content = String::new();
    let mut tool_blocks = BTreeMap::<u64, ToolBlock>::new();
    let mut finish_reason = None;
    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;
    while let Some(chunk) = stream.next().await {
        let bytes =
            chunk.map_err(|error| ApiError::LlmFailed(format!("stream chunk error: {error}")))?;
        buffer.push_str(std::str::from_utf8(&bytes).unwrap_or_default());
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim_end_matches('\r').to_string();
            buffer = buffer[pos + 1..].to_string();
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            let event: Value = serde_json::from_str(data).map_err(|error| {
                ApiError::LlmFailed(format!("invalid Anthropic stream event: {error}"))
            })?;
            match event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "message_start" => {
                    input_tokens = event
                        .pointer("/message/usage/input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or_default() as u32;
                }
                "content_block_start" => {
                    if event.pointer("/content_block/type").and_then(Value::as_str)
                        == Some("tool_use")
                    {
                        let index = event
                            .get("index")
                            .and_then(Value::as_u64)
                            .unwrap_or_default();
                        tool_blocks.insert(
                            index,
                            ToolBlock {
                                id: event
                                    .pointer("/content_block/id")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                name: event
                                    .pointer("/content_block/name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                arguments: String::new(),
                            },
                        );
                    }
                }
                "content_block_delta" => match event.pointer("/delta/type").and_then(Value::as_str)
                {
                    Some("text_delta") => {
                        if let Some(delta) = event.pointer("/delta/text").and_then(Value::as_str) {
                            content.push_str(delta);
                            if !delta.is_empty() {
                                on_chunk(delta.to_string());
                            }
                        }
                    }
                    Some("input_json_delta") => {
                        let index = event
                            .get("index")
                            .and_then(Value::as_u64)
                            .unwrap_or_default();
                        if let Some(delta) =
                            event.pointer("/delta/partial_json").and_then(Value::as_str)
                        {
                            tool_blocks
                                .entry(index)
                                .or_default()
                                .arguments
                                .push_str(delta);
                        }
                    }
                    _ => {}
                },
                "message_delta" => {
                    finish_reason = event
                        .pointer("/delta/stop_reason")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    output_tokens = event
                        .pointer("/usage/output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(output_tokens as u64) as u32;
                }
                "error" => {
                    let message = event
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Anthropic stream failed");
                    return Err(ApiError::LlmFailed(message.to_string()));
                }
                _ => {}
            }
        }
    }
    let tool_calls = tool_blocks
        .into_values()
        .filter(|block| !block.id.is_empty() && !block.name.is_empty())
        .map(|block| ToolCall {
            id: block.id,
            call_type: Some("function".to_string()),
            function: FunctionCall {
                name: block.name,
                arguments: if block.arguments.is_empty() {
                    "{}".to_string()
                } else {
                    block.arguments
                },
            },
        })
        .collect::<Vec<_>>();
    Ok(LlmResponse {
        content,
        finish_reason,
        tokens: (input_tokens > 0 || output_tokens > 0).then_some(TokenUsage {
            input_tokens,
            output_tokens,
        }),
        cached_tokens: 0,
        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
        reasoning_content: None,
        provider_items: None,
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
