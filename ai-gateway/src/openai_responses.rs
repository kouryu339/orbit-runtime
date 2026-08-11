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
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .expect("build OpenAI Responses HTTP client")
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
    mut on_chunk: F,
) -> crate::error::Result<LlmResponse>
where
    F: FnMut(String) + Send,
{
    use futures_util::StreamExt;

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
            let request = http_client()
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

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut completed_response = None;
    let mut completed_items = std::collections::BTreeMap::<u64, Value>::new();
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
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let event: Value = serde_json::from_str(data).map_err(|error| {
                ApiError::LlmFailed(format!("invalid Responses stream event: {error}"))
            })?;
            match event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "response.output_text.delta" => {
                    if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                        if !delta.is_empty() {
                            on_chunk(delta.to_string());
                        }
                    }
                }
                "response.output_item.done" => {
                    if let (Some(index), Some(item)) = (
                        event.get("output_index").and_then(Value::as_u64),
                        event.get("item").cloned(),
                    ) {
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
                    return Err(ApiError::LlmFailed(message.to_string()));
                }
                _ => {}
            }
        }
    }
    let response = completed_response.unwrap_or_else(|| {
        json!({
            "status": "completed",
            "output": completed_items.into_values().collect::<Vec<_>>()
        })
    });
    parse_response(response)
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
