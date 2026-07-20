// OpenAI-compatible chat, vision, video, and streaming adapters.
//
// This module keeps provider-facing request/response translation local to the
// gateway so higher layers can use the shared llm_gateway types.

use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};

use crate::classify::{classify_http_error, classify_network_error};
use crate::error::ApiError;
use crate::providers::ToolChoiceStyle;
use crate::retry::{retry_with_backoff, RetryPolicy};
use crate::types::{
    ChatMessage, FunctionCall, LlmResponse, TokenUsage, ToolCall, ToolDefinition, VlmResponse,
};

fn http_client() -> &'static Client {
    use once_cell::sync::OnceCell;
    static CLIENT: OnceCell<Client> = OnceCell::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .unwrap_or_else(|_| Client::new())
    })
}

fn diagnostic_excerpt(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let mut out = normalized.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

fn diagnostic_http_error_summary(text: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return diagnostic_excerpt(text, 240);
    };
    let Some(error) = value.get("error") else {
        return diagnostic_excerpt(text, 240);
    };
    let code = error
        .get("code")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let error_type = error
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let message = error
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    format!(
        "error_type={} code={} message={}",
        error_type,
        code,
        diagnostic_excerpt(message, 200)
    )
}

/// Calls an OpenAI-compatible chat completions endpoint without streaming.
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
    tool_choice_style: ToolChoiceStyle,
    force_json: bool,
) -> crate::error::Result<LlmResponse> {
    let client = http_client().clone();
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let msgs: Vec<Value> = messages
        .iter()
        .map(|m| {
            let mut msg = json!({
                "role": m.role,
                "content": m.content,
            });

            if let Some(ref tcs) = m.tool_calls {
                msg["tool_calls"] = json!(tcs
                    .iter()
                    .map(|tc| json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.function.name,
                            "arguments": tc.function.arguments,
                        }
                    }))
                    .collect::<Vec<_>>());
                if m.content.is_empty() {
                    msg["content"] = Value::Null;
                }
            }

            if let Some(ref id) = m.tool_call_id {
                msg["tool_call_id"] = json!(id);
            }

            if let Some(ref rc) = m.reasoning_content {
                if !rc.is_empty() {
                    msg["reasoning_content"] = json!(rc);
                }
            }

            msg
        })
        .collect();

    crate::diagnostics::log_provider_messages(
        "openai_compat",
        "openai_chat_completions",
        model,
        &url,
        false,
        &msgs,
        tools.len(),
    );

    let mut body = json!({
        "model": model,
        "messages": msgs,
        "stream": false,
    });

    if !tools.is_empty() {
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.function.name,
                        "description": t.function.description,
                        "parameters": t.function.parameters,
                    }
                })
            })
            .collect();
        body["tools"] = json!(tools_json);
    }

    if let Some(name) = force_tool_name {
        match tool_choice_style {
            ToolChoiceStyle::ForceName => {
                body["tool_choice"] = json!({"type": "function", "function": {"name": name}});
            }
            ToolChoiceStyle::Required => {
                body["tool_choice"] = json!("required");
            }
            ToolChoiceStyle::None => {}
        }
    }
    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    if let Some(p) = top_p {
        body["top_p"] = json!(p);
    }
    if let Some(m) = max_tokens {
        body["max_tokens"] = json!(m);
    }
    if force_json {
        body["response_format"] = json!({"type": "json_object"});
    }

    tracing::debug!(
        "LLM request model={}, messages={}, tools={}",
        model,
        messages.len(),
        tools.len()
    );

    let body_arc = std::sync::Arc::new(body);
    let url_arc = std::sync::Arc::new(url);
    let key_arc = std::sync::Arc::new(api_key.to_string());
    let model_arc = std::sync::Arc::new(model.to_string());
    let runtime_headers_arc =
        std::sync::Arc::new(crate::request_context::current_request_headers());
    crate::request_context::validate_header_transport(
        url_arc.as_ref(),
        runtime_headers_arc.as_ref(),
    )?;

    let http_resp = retry_with_backoff(RetryPolicy::default(), model, |attempt| {
        let client = client.clone();
        let body = body_arc.clone();
        let url = url_arc.clone();
        let key = key_arc.clone();
        let model = model_arc.clone();
        let runtime_headers = runtime_headers_arc.clone();
        async move {
            let mut req = client
                .post(url.as_str())
                .header("Content-Type", "application/json");
            if !key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            req = crate::request_context::apply_request_headers(req, runtime_headers.as_ref());
            let resp = req
                .json(body.as_ref())
                .send()
                .await
                .map_err(|e| classify_network_error(&e))?;
            let status = resp.status();
            if !status.is_success() {
                let headers = resp.headers().clone();
                let body_text = resp.text().await.unwrap_or_default();
                let error_summary = diagnostic_http_error_summary(&body_text);
                tracing::warn!(
                    target: "ai_gateway::http",
                    attempt = attempt + 1,
                    status = status.as_u16(),
                    model = %model,
                    url = %url,
                    error_summary = %error_summary,
                    "OpenAI-compatible HTTP request failed"
                );
                crate::diagnostics::append_line(format!(
                    "[ai-gateway http] attempt={} status={} model={} url={} error_summary={}",
                    attempt + 1,
                    status.as_u16(),
                    model,
                    url,
                    error_summary
                ));
                return Err(classify_http_error(
                    status,
                    &body_text,
                    &headers,
                    model.as_str(),
                ));
            }
            Ok(resp)
        }
    })
    .await?;

    let resp: Value = http_resp.json().await.map_err(|e| {
        ApiError::LlmFailed(format!(
            "failed to parse LLM response JSON [{}]: {e}",
            base_url
        ))
    })?;

    if let Some(err_obj) = resp.get("error") {
        let msg = err_obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown provider error");
        return Err(ApiError::LlmFailed(format!(
            "API error [{}]: {}",
            model, msg
        )));
    }

    let choices = resp["choices"].as_array();
    if choices.map(|c| c.is_empty()).unwrap_or(true) {
        return Err(ApiError::LlmFailed(format!(
            "empty choices in LLM response [{}]",
            model
        )));
    }

    let choice = &resp["choices"][0];
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

    if finish_reason.is_empty() || finish_reason == "null" {
        let message = &choice["message"];
        let has_content = message["content"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        let has_tool_calls = message
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        if !has_content && !has_tool_calls {
            return Err(ApiError::LlmFailed(
                "LLM response has no finish_reason and no content/tool_calls".into(),
            ));
        }
    }

    let message = &choice["message"];

    let content = message["content"].as_str().unwrap_or("").to_string();
    let reasoning_content = message["reasoning_content"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let tool_calls: Option<Vec<ToolCall>> = message
        .get("tool_calls")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|tc| {
                    let id = tc["id"].as_str().unwrap_or("").to_string();
                    let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                    let arguments = tc["function"]["arguments"]
                        .as_str()
                        .unwrap_or("{}")
                        .to_string();
                    if name.is_empty() {
                        return None;
                    }
                    Some(ToolCall {
                        id,
                        call_type: Some("function".to_string()),
                        function: FunctionCall { name, arguments },
                    })
                })
                .collect()
        })
        .filter(|v: &Vec<ToolCall>| !v.is_empty());

    if let Some(ref tcs) = tool_calls {
        tracing::info!(
            "LLM model={} returned {} tool calls: {}",
            model,
            tcs.len(),
            tcs.iter()
                .map(|tc| tc.function.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let tokens = resp.get("usage").map(|u| TokenUsage {
        input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
    });

    let cached_tokens = 0u32;

    Ok(LlmResponse {
        content,
        tokens,
        cached_tokens,
        tool_calls,
        reasoning_content,
    })
}

/// Calls an OpenAI-compatible vision endpoint with a local image encoded as a data URL.
pub async fn call_inner_vision(
    image_path: &str,
    prompt: &str,
    system_message: Option<&str>,
    model: &str,
    base_url: &str,
    api_key: &str,
    temperature: Option<f64>,
    max_tokens: Option<u32>,
) -> crate::error::Result<VlmResponse> {
    use base64::{engine::general_purpose, Engine as _};

    let image_data = std::fs::read(image_path)
        .map_err(|e| ApiError::VlmFailed(format!("failed to read image [{}]: {e}", image_path)))?;
    let image_base64 = general_purpose::STANDARD.encode(&image_data);

    let mime = match std::path::Path::new(image_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/jpeg",
    };

    let data_url = format!("data:{};base64,{}", mime, image_base64);

    let client = Client::new();
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let mut messages = Vec::new();
    if let Some(sys) = system_message {
        if !sys.is_empty() {
            messages.push(serde_json::json!({"role": "system", "content": sys}));
        }
    }
    {
        messages.push(serde_json::json!({
            "role": "user",
            "content": [
                {
                    "type": "image_url",
                    "image_url": { "url": data_url }
                },
                {
                    "type": "text",
                    "text": prompt
                }
            ]
        }));
    }

    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });

    if let Some(t) = temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(m) = max_tokens {
        body["max_tokens"] = serde_json::json!(m);
    }

    tracing::debug!("VLM vision request model={} image={}", model, image_path);

    let mut req = client.post(&url).header("Content-Type", "application/json");

    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }

    let resp: serde_json::Value = req
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            ApiError::VlmFailed(format!("HTTP vision request failed [{}]: {e}", base_url))
        })?
        .json()
        .await
        .map_err(|e| {
            ApiError::VlmFailed(format!(
                "failed to parse vision response JSON [{}]: {e}",
                base_url
            ))
        })?;

    if let Some(err_obj) = resp.get("error") {
        let msg = err_obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown provider error");
        return Err(ApiError::VlmFailed(format!(
            "API error [{}]: {}",
            model, msg
        )));
    }

    let content = resp["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| ApiError::VlmFailed(format!("Vision response missing content [{}]", model)))?
        .to_string();

    let tokens = resp.get("usage").map(|u| TokenUsage {
        input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(VlmResponse { content, tokens })
}

/// Calls an OpenAI-compatible vision endpoint with a local video encoded as a data URL.
pub async fn call_inner_vision_video(
    video_path: &str,
    prompt: &str,
    system_message: Option<&str>,
    model: &str,
    base_url: &str,
    api_key: &str,
    temperature: Option<f64>,
    max_tokens: Option<u32>,
) -> crate::error::Result<VlmResponse> {
    use base64::{engine::general_purpose, Engine as _};

    let video_data = tokio::fs::read(video_path)
        .await
        .map_err(|e| ApiError::VlmFailed(format!("failed to read video [{}]: {e}", video_path)))?;
    let video_base64 = general_purpose::STANDARD.encode(&video_data);

    let mime = match std::path::Path::new(video_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("avi") => "video/x-msvideo",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("flv") => "video/x-flv",
        Some("ts") => "video/mp2t",
        _ => "video/mp4",
    };

    let data_url = format!("data:{};base64,{}", mime, video_base64);

    let client = Client::new();
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let mut messages = Vec::new();
    if let Some(sys) = system_message {
        if !sys.is_empty() {
            messages.push(serde_json::json!({"role": "system", "content": sys}));
        }
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": [
            {
                "type": "video_url",
                "video_url": { "url": data_url }
            },
            {
                "type": "text",
                "text": prompt
            }
        ]
    }));

    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });

    if let Some(t) = temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(m) = max_tokens {
        body["max_tokens"] = serde_json::json!(m);
    }

    tracing::debug!("Video vision request model={} video={}", model, video_path);

    let mut req = client.post(&url).header("Content-Type", "application/json");

    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }

    let resp: serde_json::Value = req
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::VlmFailed(format!("HTTP video request failed [{}]: {e}", base_url)))?
        .json()
        .await
        .map_err(|e| {
            ApiError::VlmFailed(format!(
                "failed to parse video response JSON [{}]: {e}",
                base_url
            ))
        })?;

    if let Some(err_obj) = resp.get("error") {
        let msg = err_obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown provider error");
        return Err(ApiError::VlmFailed(format!(
            "API error [{}]: {}",
            model, msg
        )));
    }

    let content = resp["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| {
            ApiError::VlmFailed(format!("Video vision response missing content [{}]", model))
        })?
        .to_string();

    let tokens = resp.get("usage").map(|u| TokenUsage {
        input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(VlmResponse { content, tokens })
}

/// Calls an OpenAI-compatible chat completions endpoint with SSE streaming.
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
    tool_choice_style: ToolChoiceStyle,
    force_json: bool,
    mut on_chunk: F,
) -> crate::error::Result<LlmResponse>
where
    F: FnMut(String) + Send,
{
    use futures_util::StreamExt;
    use std::collections::BTreeMap;

    let client = http_client().clone();
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let msgs: Vec<Value> = messages
        .iter()
        .map(|m| {
            let mut msg = json!({ "role": m.role, "content": m.content });
            if let Some(ref tcs) = m.tool_calls {
                msg["tool_calls"] =
                    json!(tcs.iter().map(|tc| json!({
                "id": tc.id, "type": "function",
                "function": { "name": tc.function.name, "arguments": tc.function.arguments }
            })).collect::<Vec<_>>());
                if m.content.is_empty() {
                    msg["content"] = Value::Null;
                }
            }
            if let Some(ref id) = m.tool_call_id {
                msg["tool_call_id"] = json!(id);
            }
            if let Some(ref rc) = m.reasoning_content {
                if !rc.is_empty() {
                    msg["reasoning_content"] = json!(rc);
                }
            }
            msg
        })
        .collect();

    crate::diagnostics::log_provider_messages(
        "openai_compat",
        "openai_chat_completions",
        model,
        &url,
        true,
        &msgs,
        tools.len(),
    );

    let mut body = json!({ "model": model, "messages": msgs, "stream": true });

    if !tools.is_empty() {
        body["tools"] = json!(tools.iter().map(|t| json!({
            "type": "function",
            "function": { "name": t.function.name, "description": t.function.description, "parameters": t.function.parameters }
        })).collect::<Vec<_>>());
    }

    if let Some(name) = force_tool_name {
        match tool_choice_style {
            ToolChoiceStyle::ForceName => {
                body["tool_choice"] = json!({"type": "function", "function": {"name": name}});
            }
            ToolChoiceStyle::Required => {
                body["tool_choice"] = json!("required");
            }
            ToolChoiceStyle::None => {}
        }
    }
    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    if let Some(p) = top_p {
        body["top_p"] = json!(p);
    }
    if let Some(m) = max_tokens {
        body["max_tokens"] = json!(m);
    }
    if force_json {
        body["response_format"] = json!({"type": "json_object"});
    }

    let body_arc = std::sync::Arc::new(body);
    let url_arc = std::sync::Arc::new(url);
    let key_arc = std::sync::Arc::new(api_key.to_string());
    let model_arc = std::sync::Arc::new(model.to_string());
    let runtime_headers_arc =
        std::sync::Arc::new(crate::request_context::current_request_headers());
    crate::request_context::validate_header_transport(
        url_arc.as_ref(),
        runtime_headers_arc.as_ref(),
    )?;
    let resp = retry_with_backoff(RetryPolicy::default(), model, |attempt| {
        let client = client.clone();
        let body = body_arc.clone();
        let url = url_arc.clone();
        let key = key_arc.clone();
        let model = model_arc.clone();
        let runtime_headers = runtime_headers_arc.clone();
        async move {
            let mut req = client
                .post(url.as_str())
                .header("Content-Type", "application/json");
            if !key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            req = crate::request_context::apply_request_headers(req, runtime_headers.as_ref());
            let resp = req
                .json(body.as_ref())
                .send()
                .await
                .map_err(|e| classify_network_error(&e))?;
            let status = resp.status();
            if !status.is_success() {
                let headers = resp.headers().clone();
                let text = resp.text().await.unwrap_or_default();
                let error_summary = diagnostic_http_error_summary(&text);
                tracing::warn!(
                    target: "ai_gateway::http",
                    attempt = attempt + 1,
                    status = status.as_u16(),
                    model = %model,
                    url = %url,
                    error_summary = %error_summary,
                    "OpenAI-compatible streaming HTTP request failed"
                );
                crate::diagnostics::append_line(format!(
                    "[ai-gateway http] attempt={} status={} model={} url={} error_summary={}",
                    attempt + 1,
                    status.as_u16(),
                    model,
                    url,
                    error_summary
                ));
                return Err(classify_http_error(status, &text, &headers, model.as_str()));
            }
            Ok(resp)
        }
    })
    .await?;

    let mut stream = resp.bytes_stream();
    let mut full_content = String::new();
    let mut full_reasoning = String::new();
    let mut tc_map: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
    let mut buf = String::new();
    let mut arg_extractor = ArgsStreamExtractor::new();

    'outer: while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| ApiError::LlmFailed(format!("stream chunk error: {e}")))?;
        buf.push_str(std::str::from_utf8(&bytes).unwrap_or(""));

        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].trim_end_matches('\r').to_string();
            buf = buf[pos + 1..].to_string();

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                break 'outer;
            }
            if data.is_empty() {
                continue;
            }

            let Ok(val) = serde_json::from_str::<Value>(data) else {
                continue;
            };

            if let Some(err) = val.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown provider error");
                return Err(ApiError::LlmFailed(format!(
                    "stream API error [{}]: {}",
                    model, msg
                )));
            }

            let delta = &val["choices"][0]["delta"];

            if let Some(s) = delta["content"].as_str() {
                if !s.is_empty() {
                    full_content.push_str(s);
                    on_chunk(s.to_string());
                }
            }

            if let Some(s) = delta["reasoning_content"].as_str() {
                if !s.is_empty() {
                    full_reasoning.push_str(s);
                }
            }

            if let Some(tcs) = delta["tool_calls"].as_array() {
                for tc in tcs {
                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                    let entry = tc_map.entry(idx).or_default();
                    if let Some(id) = tc["id"].as_str() {
                        entry.0 = id.to_string();
                    }
                    if let Some(n) = tc["function"]["name"].as_str() {
                        entry.1 = n.to_string();
                    }
                    if let Some(a) = tc["function"]["arguments"].as_str() {
                        entry.2.push_str(a);
                        if let Some(visible) = arg_extractor.feed(a) {
                            on_chunk(visible);
                        }
                    }
                }
            }
        }
    }

    let tool_calls = if tc_map.is_empty() {
        None
    } else {
        let calls: Vec<ToolCall> = tc_map
            .into_values()
            .filter(|(_, name, _)| !name.is_empty())
            .map(|(id, name, arguments)| ToolCall {
                id,
                call_type: Some("function".to_string()),
                function: FunctionCall { name, arguments },
            })
            .collect();
        if calls.is_empty() {
            None
        } else {
            Some(calls)
        }
    };

    let reasoning_content = if full_reasoning.is_empty() {
        None
    } else {
        Some(full_reasoning)
    };
    Ok(LlmResponse {
        content: full_content,
        tokens: None,
        cached_tokens: 0,
        tool_calls,
        reasoning_content,
    })
}

// ============================================================================
// FC argument stream extraction.
//
// Some providers stream function-call arguments as partial JSON. This helper
// watches selected string fields inside tool_call.function.arguments and emits
// incremental text for user-facing prompt/result/question fields.
//
// Example target shape: {"to_state":"asking","prompt":"..."}
//
// Keep this extractor conservative: malformed chunks should not break streaming.
// ============================================================================
// ============================================================================

/// Fields to extract from streamed function-call arguments.
const STREAM_FIELDS: &[&str] = &["prompt", "result", "question"];

#[derive(Debug, Default)]
enum ExtractState {
    #[default]
    /// Looking for a target field name.
    Scanning,
    /// Field was found; waiting for the value string.
    AwaitingValue,
    /// Reading a JSON string value.
    InString,
    /// Previous character was a backslash escape.
    Escaped,
    /// Extraction is complete.
    Done,
}

struct ArgsStreamExtractor {
    state: ExtractState,
    full: String,
    cursor: usize,
}

impl ArgsStreamExtractor {
    fn new() -> Self {
        Self {
            state: ExtractState::Scanning,
            full: String::new(),
            cursor: 0,
        }
    }

    fn feed(&mut self, delta: &str) -> Option<String> {
        self.full.push_str(delta);
        let mut out = String::new();

        loop {
            match self.state {
                ExtractState::Scanning => {
                    let remaining = &self.full[self.cursor..];
                    let mut found = false;
                    for &field in STREAM_FIELDS {
                        let pat = format!("\"{}\"", field);
                        if let Some(pos) = remaining.find(&pat) {
                            self.cursor += pos + pat.len();
                            self.state = ExtractState::AwaitingValue;
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        if self.full.len() > 20 {
                            self.cursor = self.full.len() - 20;
                        }
                        break;
                    }
                }
                ExtractState::AwaitingValue => {
                    let remaining = &self.full[self.cursor..];
                    let mut advanced = false;
                    for (i, ch) in remaining.char_indices() {
                        if ch == '"' {
                            self.cursor += i + '"'.len_utf8();
                            self.state = ExtractState::InString;
                            advanced = true;
                            break;
                        } else if ch.is_whitespace() || ch == ':' {
                        } else {
                            self.state = ExtractState::Scanning;
                            advanced = true;
                            break;
                        }
                    }
                    if !advanced {
                        break;
                    }
                }
                ExtractState::InString => {
                    let remaining = &self.full[self.cursor..];
                    let mut end_of_data = true;
                    for (i, ch) in remaining.char_indices() {
                        match ch {
                            '\\' => {
                                self.cursor += i + '\\'.len_utf8();
                                self.state = ExtractState::Escaped;
                                end_of_data = false;
                                break;
                            }
                            '"' => {
                                self.cursor += i + '"'.len_utf8();
                                self.state = ExtractState::Done;
                                end_of_data = false;
                                break;
                            }
                            c => {
                                out.push(c);
                            }
                        }
                    }
                    if end_of_data {
                        self.cursor = self.full.len();
                        break;
                    }
                }
                ExtractState::Escaped => {
                    let remaining = &self.full[self.cursor..];
                    if remaining.is_empty() {
                        break;
                    }
                    let ch = remaining.chars().next().unwrap();
                    self.cursor += ch.len_utf8();
                    let decoded = match ch {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '"' => '"',
                        '\\' => '\\',
                        '/' => '/',
                        _ => ch,
                    };
                    out.push(decoded);
                    self.state = ExtractState::InString;
                }
                ExtractState::Done => {
                    break;
                }
            }
        }

        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}
