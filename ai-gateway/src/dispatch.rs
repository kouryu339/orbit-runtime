//!
//! 数据流：
//!   call_llm(model_id) → key_store.get(model_id)
//!   → model_name + provider_id
//!   → key_store.resolve_provider(provider_id) → api_key + base_url
//!   → builtin providers 表获取默认 base_url（用户覆盖时用用户的）

use crate::config::ApiParadigm;
use crate::error::{ApiError, FatalError};
use crate::providers::{self, ApiFormat, ToolChoiceStyle};
use crate::types::{ChatMessage, LlmResponse, ToolDefinition};

/// 查询指定模型的 tool_choice 支持方式
pub fn model_tool_choice_style(model_name: Option<&str>) -> ToolChoiceStyle {
    providers::resolve(model_name.unwrap_or("")).tool_choice_style
}

/// 查询指定模型是否支持 FC
pub fn model_supports_tool_choice(model_name: Option<&str>) -> bool {
    providers::resolve(model_name.unwrap_or("")).tool_choice_style != ToolChoiceStyle::None
}

async fn dispatch(
    messages: &[ChatMessage],
    tools: &[ToolDefinition],
    model_id: u32,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    force_tool_name: Option<&str>,
    force_json: bool,
) -> crate::error::Result<LlmResponse> {
    let entry = crate::key_store::get(model_id).ok_or_else(|| {
        ApiError::Fatal(FatalError::config_error(format!(
            "未找到 model_id={} 的配置，请先在「设置 → AI 配置」中启用",
            model_id
        )))
    })?;

    let provider = providers::resolve(&entry.model_name);

    let provider_runtime = crate::key_store::resolve_provider_runtime(entry.provider_uid)
        .ok_or_else(|| {
            ApiError::Fatal(FatalError::config_error(format!(
                "model_uid={} (provider_uid={}) 未配置 API Key，请先在设置中填写",
                model_id, entry.provider_uid
            )))
        })?;

    let base_url = if provider_runtime.base_url.is_empty() {
        provider.base_url.clone()
    } else {
        provider_runtime.base_url.clone()
    };
    let api_key = provider_runtime.api_key;
    let api_format = effective_api_format(provider.format, provider_runtime.api_paradigm)?;
    let api_format_label = match api_format {
        ApiFormat::OpenAI => "openai_chat_completions",
        ApiFormat::Anthropic => "anthropic_messages",
    };

    tracing::debug!(
        "路由到 {} [{}]: {}",
        base_url,
        match api_format {
            ApiFormat::OpenAI => "OpenAI",
            ApiFormat::Anthropic => "Anthropic",
        },
        entry.model_name
    );
    crate::diagnostics::log_chat_messages(
        "dispatch",
        model_id,
        &entry.model_name,
        api_format_label,
        messages,
        tools.len(),
    );

    match api_format {
        ApiFormat::OpenAI => {
            crate::openai_compat::call_inner(
                messages,
                tools,
                &entry.model_name,
                &base_url,
                &api_key,
                temperature,
                top_p,
                max_tokens,
                force_tool_name,
                provider.tool_choice_style,
                force_json,
            )
            .await
        }
        ApiFormat::Anthropic => {
            crate::anthropic_compat::call_inner(
                messages,
                tools,
                &entry.model_name,
                &base_url,
                &api_key,
                temperature,
                top_p,
                max_tokens,
                force_tool_name,
            )
            .await
        }
    }
}

fn effective_api_format(
    builtin_format: ApiFormat,
    api_paradigm: Option<ApiParadigm>,
) -> crate::error::Result<ApiFormat> {
    match api_paradigm {
        None => Ok(builtin_format),
        Some(ApiParadigm::AnthropicMessages) => Ok(ApiFormat::Anthropic),
        Some(ApiParadigm::OpenAiChatCompletions) => Ok(ApiFormat::OpenAI),
        Some(ApiParadigm::OpenAiResponses) => Err(ApiError::Fatal(FatalError::config_error(
            "api_paradigm=openai_responses is reserved but not supported by ai-gateway yet",
        ))),
    }
}

pub async fn call_llm(
    model_id: u32,
    messages: &[ChatMessage],
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
) -> crate::error::Result<LlmResponse> {
    dispatch(
        messages,
        &[],
        model_id,
        temperature,
        top_p,
        max_tokens,
        None,
        false,
    )
    .await
}

///
/// `force_json=true` 时网关层保证返回合法 JSON：若模型返回非 JSON，
/// 自动注入纠正消息重试一次，上层无需关心。
pub async fn call_llm_cancellable(
    model_id: u32,
    messages: &[ChatMessage],
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    cancel: tokio_util::sync::CancellationToken,
) -> crate::error::Result<LlmResponse> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(crate::error::ApiError::Cancelled),
        r = dispatch(messages, &[], model_id, temperature, top_p, max_tokens, None, false) => r,
    }
}

///
/// **重要变更（feat/line-protocol）**：
/// 网关层不再做任何 JSON 修复 / 自动重试 / `response_format: json_object` 强制。
/// 这些职责全部上移到 `ai-assistant`：
/// - JSON 解析与兜底：`ai_assistant::decision::parse_ai_response`（7 层修复链）
/// - 行式协议解析：`ai_assistant::decision_line::parse_line_protocol`
/// - 失败重试：thinking 状态自带 3 次循环 + 错误注入
///
/// 这样做的原因：
/// 1. 老逻辑会在第一次解析失败时悄悄发起一次额外 LLM 调用（force_json=true 重试），
///    导致 ASK/RESULT 这类长文本场景延迟翻倍（用户感知"网关卡死"）。
/// 2. `response_format: json_object` 与行式协议互斥，灰度切换时会两边打架。
/// 3. 上层有更丰富的上下文（cache、turn_id、协议选择），更适合做错误回路。
///
/// 网关层只保留"传输无感知"行为：发请求、等响应、取消即取消。
pub async fn call_llm_json_cancellable(
    model_id: u32,
    messages: &[ChatMessage],
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    cancel: tokio_util::sync::CancellationToken,
) -> crate::error::Result<LlmResponse> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(crate::error::ApiError::Cancelled),
        r = dispatch(messages, &[], model_id, temperature, top_p, max_tokens, None, false) => r,
    }
}

pub async fn call_llm_with_tools(
    model_id: u32,
    messages: &[ChatMessage],
    tools: &[ToolDefinition],
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
) -> crate::error::Result<LlmResponse> {
    dispatch(
        messages,
        tools,
        model_id,
        temperature,
        top_p,
        max_tokens,
        None,
        false,
    )
    .await
}

///
/// 与 `call_llm_cancellable` 相同，但额外传入 `tools`：
/// 模型可以自由决定是否调用工具（`tool_choice` 为 auto，不强制）。
///
/// **返回值语义**：
/// - `resp.tool_calls` 非空 → 模型决定执行工具
/// - `resp.tool_calls` 为空 → 模型选择直接用文本回答
///
/// 上层（`ai-assistant` thinking 状态）据此走执行分支或对话分支，
/// 不再依赖行式输出头。
pub async fn call_llm_with_tools_cancellable(
    model_id: u32,
    messages: &[ChatMessage],
    tools: &[ToolDefinition],
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    cancel: tokio_util::sync::CancellationToken,
) -> crate::error::Result<LlmResponse> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(crate::error::ApiError::Cancelled),
        r = dispatch(messages, tools, model_id, temperature, top_p, max_tokens, None, false) => r,
    }
}

// ============================================================================
// 接口级保底：FC arguments 合法性校验 + 自动重试
//
// call_llm_decide / call_llm_decide_streaming 对调用方承诺：
//   返回的 tool_call.arguments 一定是合法 JSON。
// 若第一次调用返回非法 arguments，追加纠正消息后自动重试一次（dispatch 层内部，上层无感知）。
// ============================================================================

/// 检查 tool_call arguments 是否合法 JSON，或者模型根本没有返回 tool_calls。
/// Ok(()) = 合法；Err(raw) = 不合法或缺失，附原始内容（缺失时为 content）用于构建纠正消息。
fn check_fc_args(resp: &LlmResponse) -> Result<(), String> {
    // 没有 tool_calls：模型直接输出了自然语言，需要纠正
    let Some(ref tcs) = resp.tool_calls else {
        return Err(resp.content.chars().take(80).collect());
    };
    let Some(tc) = tcs.first() else {
        return Err(resp.content.chars().take(80).collect());
    };
    match serde_json::from_str::<serde_json::Value>(&tc.function.arguments) {
        Ok(_) => Ok(()),
        Err(_) => Err(tc.function.arguments.clone()),
    }
}

/// 构造追加到 messages 末尾的纠正消息
fn correction_message(bad_args: &str) -> ChatMessage {
    let preview = &bad_args[..bad_args.len().min(80)];
    ChatMessage {
        role: "user".to_string(),
        content: format!(
            "[系统] 你上一次调用工具时 arguments 不是合法 JSON（收到：{:?}）。\
             请立即重新调用，arguments 必须是合法 JSON 对象，\
             例如 {{\"to_state\":\"result\",\"result\":\"你的回复\"}}。",
            preview
        ),
        cache_control: false,
        tool_call_id: None,
        name: None,
        tool_calls: None,
        reasoning_content: None,
    }
}

/// 强制工具调用（接口级保底：arguments 非法 JSON 自动重试一次）
pub async fn call_llm_decide(
    model_id: u32,
    messages: &[ChatMessage],
    tool: &ToolDefinition,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    force_json: bool,
) -> crate::error::Result<LlmResponse> {
    let force_name = tool.function.name.clone();

    let resp = dispatch(
        messages,
        std::slice::from_ref(tool),
        model_id,
        temperature,
        top_p,
        max_tokens,
        Some(&force_name),
        force_json,
    )
    .await?;

    if let Err(bad_args) = check_fc_args(&resp) {
        tracing::warn!(
            "[dispatch] call_llm_decide: arguments 非法 JSON，自动重试一次。bad={:?}",
            &bad_args[..bad_args.len().min(80)]
        );
        let mut msgs2 = messages.to_vec();
        msgs2.push(correction_message(&bad_args));
        dispatch(
            &msgs2,
            std::slice::from_ref(tool),
            model_id,
            temperature,
            top_p,
            max_tokens,
            Some(&force_name),
            force_json,
        )
        .await
    } else {
        Ok(resp)
    }
}

/// 强制工具调用（支持取消，接口级保底：arguments 非法 JSON 自动重试一次）
///
/// `cancel` 被触发时立即返回 `Err(ApiError::Cancelled)`，HTTP 请求被 drop 中断。
pub async fn call_llm_decide_cancellable(
    model_id: u32,
    messages: &[ChatMessage],
    tool: &ToolDefinition,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    force_json: bool,
    cancel: tokio_util::sync::CancellationToken,
) -> crate::error::Result<LlmResponse> {
    let force_name = tool.function.name.clone();

    // 第一次调用：race cancel 和 dispatch
    let resp = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(crate::error::ApiError::Cancelled),
        r = dispatch(
            messages, std::slice::from_ref(tool),
            model_id, temperature, top_p, max_tokens,
            Some(&force_name), force_json,
        ) => r?,
    };

    if let Err(bad_args) = check_fc_args(&resp) {
        tracing::warn!(
            "[dispatch] call_llm_decide_cancellable: arguments 非法 JSON，自动重试一次。bad={:?}",
            &bad_args[..bad_args.len().min(80)]
        );
        let mut msgs2 = messages.to_vec();
        msgs2.push(correction_message(&bad_args));
        // 重试也参与 cancel 竞争
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(crate::error::ApiError::Cancelled),
            r = dispatch(
                &msgs2, std::slice::from_ref(tool),
                model_id, temperature, top_p, max_tokens,
                Some(&force_name), force_json,
            ) => r,
        }
    } else {
        Ok(resp)
    }
}

/// 流式强制工具调用（接口级保底：arguments 非法 JSON 自动降级非流式重试一次）
///
/// 与 `call_llm_decide` 完全相同，只是改用 SSE streaming。
/// `on_chunk` 每个 content delta 回调一次，FC tool_calls 合并后随返回值返回。
///
/// **注意**：streaming 模式仅支持 OpenAI 兼容格式提供商；
/// Anthropic 格式提供商暂时降级为普通调用（无 on_chunk 回调）。
/// **重试时**降级为非流式（流已结束，on_chunk 不再调用），上层负责处理流重置。
pub async fn call_llm_decide_streaming<F>(
    model_id: u32,
    messages: &[ChatMessage],
    tool: &ToolDefinition,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    force_json: bool,
    on_chunk: F,
) -> crate::error::Result<LlmResponse>
where
    F: FnMut(String) + Send,
{
    let entry = crate::key_store::get(model_id).ok_or_else(|| {
        ApiError::Fatal(FatalError::config_error(format!(
            "未找到 model_id={} 的配置",
            model_id
        )))
    })?;

    let provider = providers::resolve(&entry.model_name);

    let provider_runtime = crate::key_store::resolve_provider_runtime(entry.provider_uid)
        .ok_or_else(|| {
            ApiError::Fatal(FatalError::config_error(format!(
                "model_uid={} 未配置 API Key",
                model_id
            )))
        })?;

    let base_url = if provider_runtime.base_url.is_empty() {
        provider.base_url.clone()
    } else {
        provider_runtime.base_url.clone()
    };
    let api_key = provider_runtime.api_key;
    let api_format = effective_api_format(provider.format, provider_runtime.api_paradigm)?;

    let force_name = tool.function.name.clone();

    let resp = match api_format {
        ApiFormat::OpenAI => {
            crate::openai_compat::call_inner_streaming(
                messages,
                std::slice::from_ref(tool),
                &entry.model_name,
                &base_url,
                &api_key,
                temperature,
                top_p,
                max_tokens,
                Some(&force_name),
                provider.tool_choice_style,
                force_json,
                on_chunk,
            )
            .await?
        }
        ApiFormat::Anthropic => {
            tracing::warn!("Anthropic 格式不支持流式，降级为普通调用");
            crate::anthropic_compat::call_inner(
                messages,
                std::slice::from_ref(tool),
                &entry.model_name,
                &base_url,
                &api_key,
                temperature,
                top_p,
                max_tokens,
                Some(&force_name),
            )
            .await?
        }
    };

    if let Err(bad_args) = check_fc_args(&resp) {
        tracing::warn!(
            "[dispatch] call_llm_decide_streaming: arguments 非法 JSON，降级非流式重试一次。bad={:?}",
            &bad_args[..bad_args.len().min(80)]
        );
        let mut msgs2 = messages.to_vec();
        msgs2.push(correction_message(&bad_args));
        dispatch(
            &msgs2,
            std::slice::from_ref(tool),
            model_id,
            temperature,
            top_p,
            max_tokens,
            Some(&force_name),
            force_json,
        )
        .await
    } else {
        Ok(resp)
    }
}

/// 流式强制工具调用（支持取消）
///
/// 与 `call_llm_decide_streaming` 相同，但额外接受一个 `CancellationToken`。
/// cancel 触发时立即返回 `Err(ApiError::Cancelled)`。
pub async fn call_llm_decide_streaming_cancellable<F>(
    model_id: u32,
    messages: &[ChatMessage],
    tool: &ToolDefinition,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
    force_json: bool,
    on_chunk: F,
    cancel: tokio_util::sync::CancellationToken,
) -> crate::error::Result<LlmResponse>
where
    F: FnMut(String) + Send,
{
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(crate::error::ApiError::Cancelled),
        r = call_llm_decide_streaming(
            model_id, messages, tool,
            temperature, top_p, max_tokens,
            force_json, on_chunk,
        ) => r,
    }
}
