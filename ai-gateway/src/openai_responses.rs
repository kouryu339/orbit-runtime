//! OpenAI Responses API adapter.
//!
//! Responses output items are retained verbatim so a following request can
//! replay reasoning and function-call protocol state without flattening it
//! into chat messages.

use once_cell::sync::OnceCell;
use reqwest::Client;
use serde_json::{json, Value};

use crate::classify::{classify_http_error, classify_network_error, json_response_or_error};
use crate::retry::{retry_with_backoff, RetryPolicy};
use crate::types::{ChatMessage, LlmResponse, TokenUsage, ToolCall, ToolDefinition};
use crate::ApiError;

fn http_client() -> &'static Client {
    static CLIENT: OnceCell<Client> = OnceCell::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .expect("build OpenAI Responses HTTP client")
    })
}

fn streaming_http_client() -> &'static Client {
    static CLIENT: OnceCell<Client> = OnceCell::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(crate::stream_control::TOTAL_TIMEOUT)
            .connect_timeout(crate::stream_control::CONNECT_TIMEOUT)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .expect("build OpenAI Responses streaming HTTP client")
    })
}

fn message_input(message: &ChatMessage) -> Vec<Value> {
    if let Some(items) = message
        .provider_items
        .as_ref()
        .filter(|items| !items.is_empty())
    {
        return items.clone();
    }
    if message.role == "tool" {
        if let Some(call_id) = message.tool_call_id.as_deref() {
            return vec![json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": message.content,
            })];
        }
    }
    if message.role == "assistant" {
        if let Some(calls) = message
            .tool_calls
            .as_ref()
            .filter(|calls| !calls.is_empty())
        {
            let mut items = Vec::new();
            if !message.content.trim().is_empty() {
                items.push(json!({"role": "assistant", "content": message.content}));
            }
            items.extend(calls.iter().map(|call| {
                json!({
                    "type": "function_call",
                    "call_id": call.id,
                    "name": call.function.name,
                    "arguments": call.function.arguments,
                })
            }));
            return items;
        }
    }
    vec![json!({"role": message.role, "content": message.content})]
}

pub(crate) fn build_request_body(
    messages: &[ChatMessage],
    tools: &[ToolDefinition],
    model: &str,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    force_tool_name: Option<&str>,
) -> Value {
    let input = messages.iter().flat_map(message_input).collect::<Vec<_>>();
    let mut body = json!({"model": model, "input": input});
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    let mut definition = json!({
                        "type": "function",
                        "name": tool.function.name,
                        "description": tool.function.description,
                        "parameters": tool.function.parameters,
                    });
                    if let Some(strict) = tool.function.strict {
                        definition["strict"] = json!(strict);
                    }
                    definition
                })
                .collect(),
        );
    }
    if let Some(name) = force_tool_name {
        body["tool_choice"] = json!({"type": "function", "name": name});
    }
    if let Some(value) = temperature {
        body["temperature"] = json!(value);
    }
    if let Some(value) = top_p {
        body["top_p"] = json!(value);
    }
    if let Some(value) = max_tokens {
        body["max_output_tokens"] = json!(value);
    }
    body
}

pub(crate) fn parse_response(response: Value) -> crate::error::Result<LlmResponse> {
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| ApiError::LlmFailed("Responses response is missing output items".into()))?;
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    for item in &output {
        match item.get("type").and_then(Value::as_str).unwrap_or_default() {
            "message" => {
                if let Some(blocks) = item.get("content").and_then(Value::as_array) {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) == Some("output_text") {
                            if let Some(text) = block.get("text").and_then(Value::as_str) {
                                content.push_str(text);
                            }
                        }
                    }
                }
            }
            "function_call" => {
                let call_id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                if !call_id.is_empty() && !name.is_empty() {
                    tool_calls.push(ToolCall::function(call_id, name, arguments));
                }
            }
            _ => {}
        }
    }
    let input_tokens = response
        .pointer("/usage/input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let output_tokens = response
        .pointer("/usage/output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let cached_tokens = response
        .pointer("/usage/input_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let status = response
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(LlmResponse {
        content,
        finish_reason: status,
        tokens: (input_tokens > 0 || output_tokens > 0).then_some(TokenUsage {
            input_tokens,
            output_tokens,
        }),
        cached_tokens,
        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
        reasoning_content: None,
        provider_items: Some(output),
    })
}

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
    let url = format!("{}/responses", base_url.trim_end_matches('/'));
    let body = std::sync::Arc::new(build_request_body(
        messages,
        tools,
        model,
        temperature,
        top_p,
        max_tokens,
        force_tool_name,
    ));
    let key = std::sync::Arc::new(api_key.to_string());
    let url = std::sync::Arc::new(url);
    let runtime_headers = std::sync::Arc::new(crate::request_context::current_request_headers());
    crate::request_context::validate_header_transport(url.as_ref(), runtime_headers.as_ref())?;
    let response = retry_with_backoff(RetryPolicy::default(), model, |_attempt| {
        let body = body.clone();
        let key = key.clone();
        let url = url.clone();
        let runtime_headers = runtime_headers.clone();
        async move {
            let request = http_client()
                .post(url.as_str())
                .bearer_auth(key.as_str())
                .json(body.as_ref());
            let request = crate::request_context::apply_request_headers(request, &runtime_headers);
            let response = request
                .send()
                .await
                .map_err(|error| classify_network_error(&error))?;
            json_response_or_error(response, model).await
        }
    })
    .await?;
    parse_response(response)
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
    require_tool_call: bool,
    mut on_event: F,
) -> crate::error::Result<LlmResponse>
where
    F: FnMut(crate::types::LlmStreamEvent) + Send,
{
    let request_started = std::time::Instant::now();

    let url = format!("{}/responses", base_url.trim_end_matches('/'));
    let mut body = build_request_body(
        messages,
        tools,
        model,
        temperature,
        top_p,
        max_tokens,
        force_tool_name,
    );
    if require_tool_call && force_tool_name.is_none() {
        body["tool_choice"] = json!("required");
    }
    body["stream"] = json!(true);
    let body = std::sync::Arc::new(body);
    let key = std::sync::Arc::new(api_key.to_string());
    let url = std::sync::Arc::new(url);
    let model_owned = std::sync::Arc::new(model.to_string());
    let runtime_headers = std::sync::Arc::new(crate::request_context::current_request_headers());
    crate::request_context::validate_header_transport(url.as_ref(), runtime_headers.as_ref())?;
    let response = retry_with_backoff(RetryPolicy::default(), model, |attempt| {
        let body = body.clone();
        let key = key.clone();
        let url = url.clone();
        let model = model_owned.clone();
        let runtime_headers = runtime_headers.clone();
        async move {
            let request = streaming_http_client()
                .post(url.as_str())
                .bearer_auth(key.as_str())
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
                "OpenAI Responses streaming request failed"
            );
            Err(classify_http_error(status, &text, &headers, &model))
        }
    })
    .await?;

    crate::diagnostics::append_line(format!(
        "[ai-gateway response] {}",
        json!({
            "phase": "headers_received",
            "stream": true,
            "provider_api": "openai_responses",
            "model": model,
            "elapsed_ms": request_started.elapsed().as_millis() as u64,
        })
    ));

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut completed_response = None;
    let mut completed_items = std::collections::BTreeMap::<u64, Value>::new();
    let mut first_event_logged = false;
    let mut chunk_count = 0u64;
    let mut response_bytes = 0u64;
    loop {
        let chunk = match crate::stream_control::next_with_idle_timeout(&mut stream).await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(()) => {
                crate::diagnostics::append_line(format!(
                    "[ai-gateway response] {}",
                    json!({
                        "phase": "stream_idle_timeout",
                        "stream": true,
                        "provider_api": "openai_responses",
                        "model": model,
                        "elapsed_ms": request_started.elapsed().as_millis() as u64,
                        "idle_timeout_ms": crate::stream_control::IDLE_TIMEOUT.as_millis() as u64,
                        "first_event_received": first_event_logged,
                        "chunk_count": chunk_count,
                        "response_bytes": response_bytes,
                        "partial_output_item_count": completed_items.len(),
                    })
                ));
                return Err(ApiError::LlmFailed(format!(
                    "stream produced no data for {} seconds",
                    crate::stream_control::IDLE_TIMEOUT.as_secs()
                )));
            }
        };
        let bytes = match chunk {
            Ok(bytes) => bytes,
            Err(error) => {
                crate::diagnostics::append_line(format!(
                    "[ai-gateway response] {}",
                    json!({
                        "phase": if error.is_timeout() { "stream_total_timeout" } else { "stream_error" },
                        "stream": true,
                        "provider_api": "openai_responses",
                        "model": model,
                        "elapsed_ms": request_started.elapsed().as_millis() as u64,
                        "first_event_received": first_event_logged,
                        "chunk_count": chunk_count,
                        "response_bytes": response_bytes,
                        "total_timeout_ms": crate::stream_control::TOTAL_TIMEOUT.as_millis() as u64,
                        "partial_output_item_count": completed_items.len(),
                        "error": error.to_string(),
                    })
                ));
                return Err(ApiError::LlmFailed(format!("stream chunk error: {error}")));
            }
        };
        chunk_count = chunk_count.saturating_add(1);
        response_bytes = response_bytes.saturating_add(bytes.len() as u64);
        buffer.push_str(std::str::from_utf8(&bytes).unwrap_or_default());
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim_end_matches('\r').to_string();
            buffer = buffer[pos + 1..].to_string();
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let event: Value = serde_json::from_str(data).map_err(|error| {
                ApiError::LlmFailed(format!("invalid Responses stream event: {error}"))
            })?;
            if !first_event_logged {
                first_event_logged = true;
                crate::diagnostics::append_line(format!(
                    "[ai-gateway response] {}",
                    json!({
                        "phase": "first_stream_event",
                        "stream": true,
                        "provider_api": "openai_responses",
                        "model": model,
                        "ttft_ms": request_started.elapsed().as_millis() as u64,
                        "chunk_count": chunk_count,
                        "response_bytes": response_bytes,
                    })
                ));
            }
            match event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "response.output_text.delta" => {
                    if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                        if !delta.is_empty() {
                            on_event(crate::types::LlmStreamEvent::TextDelta {
                                text: delta.to_string(),
                            });
                        }
                    }
                }
                "response.output_item.added" => {
                    let index = event
                        .get("output_index")
                        .and_then(Value::as_u64)
                        .unwrap_or_default();
                    if let Some(item) = event.get("item").filter(|item| {
                        item.get("type").and_then(Value::as_str) == Some("function_call")
                    }) {
                        on_event(crate::types::LlmStreamEvent::ToolCallDelta {
                            index,
                            call_id: item
                                .get("call_id")
                                .or_else(|| item.get("id"))
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            name: item.get("name").and_then(Value::as_str).map(str::to_string),
                            argument_bytes: 0,
                        });
                    }
                }
                "response.function_call_arguments.delta" => {
                    let index = event
                        .get("output_index")
                        .and_then(Value::as_u64)
                        .unwrap_or_default();
                    let argument_bytes = event
                        .get("delta")
                        .and_then(Value::as_str)
                        .map(str::len)
                        .unwrap_or_default();
                    on_event(crate::types::LlmStreamEvent::ToolCallDelta {
                        index,
                        call_id: event
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        name: event
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        argument_bytes,
                    });
                }
                "response.output_item.done" => {
                    if let (Some(index), Some(item)) = (
                        event.get("output_index").and_then(Value::as_u64),
                        event.get("item").cloned(),
                    ) {
                        if item.get("type").and_then(Value::as_str) == Some("function_call") {
                            on_event(crate::types::LlmStreamEvent::ToolCallCompleted {
                                index,
                                call_id: item
                                    .get("call_id")
                                    .or_else(|| item.get("id"))
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                name: item.get("name").and_then(Value::as_str).map(str::to_string),
                            });
                        }
                        completed_items.insert(index, item);
                    }
                }
                "response.completed" => {
                    completed_response = event.get("response").cloned();
                }
                "response.failed" | "error" => {
                    let message = event
                        .pointer("/response/error/message")
                        .or_else(|| event.pointer("/error/message"))
                        .or_else(|| event.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("Responses stream failed");
                    crate::diagnostics::append_line(format!(
                        "[ai-gateway response] {}",
                        json!({
                            "phase": "provider_stream_error",
                            "stream": true,
                            "provider_api": "openai_responses",
                            "model": model,
                            "elapsed_ms": request_started.elapsed().as_millis() as u64,
                            "chunk_count": chunk_count,
                            "response_bytes": response_bytes,
                            "error": message,
                        })
                    ));
                    return Err(ApiError::LlmFailed(message.to_string()));
                }
                _ => {}
            }
        }
    }
    let Some(response) = completed_response else {
        crate::diagnostics::append_line(format!(
            "[ai-gateway response] {}",
            json!({
                "phase": "stream_incomplete",
                "stream": true,
                "provider_api": "openai_responses",
                "model": model,
                "elapsed_ms": request_started.elapsed().as_millis() as u64,
                "chunk_count": chunk_count,
                "response_bytes": response_bytes,
                "completed_output_item_count": completed_items.len(),
            })
        ));
        return Err(ApiError::LlmFailed(
            "Responses stream ended before response.completed".to_string(),
        ));
    };
    let parsed = parse_response(response)?;
    crate::diagnostics::append_line(format!(
        "[ai-gateway response] {}",
        json!({
            "phase": "completed",
            "stream": true,
            "provider_api": "openai_responses",
            "model": model,
            "elapsed_ms": request_started.elapsed().as_millis() as u64,
            "first_event_received": first_event_logged,
            "chunk_count": chunk_count,
            "response_bytes": response_bytes,
            "content_chars": parsed.content.chars().count(),
            "tool_call_count": parsed.tool_calls.as_ref().map(Vec::len).unwrap_or(0),
        })
    ));
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FunctionDefinition, ToolDefinition};

    #[test]
    fn keeps_function_call_items_and_builds_output_item() {
        let response = parse_response(json!({
            "status": "completed",
            "output": [{
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "Read",
                "arguments": "{\"path\":\"a\"}"
            }],
            "usage": {"input_tokens": 10, "output_tokens": 4}
        }))
        .unwrap();
        assert_eq!(response.tool_calls.as_ref().unwrap()[0].id, "call_1");
        let mut assistant =
            ChatMessage::assistant_with_tool_calls(response.tool_calls.clone().unwrap());
        assistant.provider_items = response.provider_items;
        let tool = ChatMessage::tool_with_id("ok", "call_1", "Read");
        let body = build_request_body(&[assistant, tool], &[], "gpt-test", None, None, None, None);
        assert_eq!(body["input"][0]["type"], "function_call");
        assert_eq!(body["input"][1]["type"], "function_call_output");
    }

    #[test]
    fn emits_flat_responses_function_definition() {
        let tool = ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "Read".into(),
                description: "read".into(),
                parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
                strict: Some(true),
            },
        };
        let body = build_request_body(&[], &[tool], "gpt-test", None, None, None, None);
        assert_eq!(body["tools"][0]["name"], "Read");
        assert_eq!(body["tools"][0]["strict"], true);
        assert!(body["tools"][0].get("function").is_none());
    }
}
