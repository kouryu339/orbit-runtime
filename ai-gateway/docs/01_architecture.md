# 1 ai-gateway 架构与错误处理

更新时间：2026-05-04

`ai-gateway` 是模型调用网关。它的目标是把不同厂商的 HTTP API、鉴权方式、工具调用格式和错误响应收敛成一套 `llm_gateway` Rust API，供 `ai-assistant` 和 corework 节点调用。

代码入口：

- `src/dispatch.rs`：LLM 模型 id 到 provider/backend 的统一分发。
- `src/openai_compat.rs`：OpenAI Chat Completions 兼容内核，覆盖 DeepSeek、Qwen compatible-mode、OpenAI、豆包、MiniMax、Kimi、GLM、Yi、Step、Ollama 等。
- `src/anthropic_compat.rs`：Anthropic Messages API 兼容内核。
- `src/providers.rs`：静态 provider/model 路由规则。
- `src/config.rs` + `src/key_store.rs`：用户配置、model_uid/provider_uid 索引和 API key resolver。
- `src/error.rs` + `src/classify.rs` + `src/retry.rs`：结构化错误分类和重试。
- `src/vlm.rs`、`src/asr.rs`、`src/ocr.rs`：多模态能力入口。
- `src/nodes/`：corework `#[buns_system]` 节点封装。

## 1.1 LLM 调用链路

```text
call_llm / call_llm_with_tools / call_llm_decide
  -> dispatch(model_uid)
  -> key_store::get(model_uid)
  -> key_store::resolve_provider(provider_uid)
  -> providers::resolve(model_name)
  -> conversation request context headers
  -> openai_compat::call_inner 或 anthropic_compat::call_inner
  -> retry_with_backoff
  -> classify_http_error / classify_network_error
  -> LlmResponse 或 ApiError
```

`dispatch.rs` 只处理文本 LLM。VLM/ASR/OCR 当前仍主要走环境变量和各自的 provider resolver，不走 `model_uid/provider_uid` 配置索引。

## 1.2 Conversation 请求头上下文

`ai-gateway` 支持从 runtime 的 conversation 请求上下文中读取本轮 LLM 调用需要附加的 HTTP headers。OpenAI-compatible 和 Anthropic-compatible adapter 会在真正发出 HTTP 请求前追加这些 headers。

这个能力用于集成式部署：宿主应用可能不暴露静态 API key，而是把用户或会话 token 作为鉴权凭据交给 runtime，例如：

```json
{
  "Authorization": "Bearer runtime-session-token",
  "X-User-Id": "user-a"
}
```

边界规则：

- 宿主或 runtime 决定某个 conversation 带哪些 headers。
- `ai-gateway` 只在当前 async 请求上下文中转发这些 headers。
- headers 不写入 provider config、ledger、snapshot 或全局状态。
- `Content-Type` 由 provider adapter 控制，不允许被上下文 headers 覆盖。

当存在上下文 headers 时，非本机回环端点默认要求 HTTPS。本机 `localhost`、`127.0.0.1`、`[::1]` 的 HTTP 可用于开发调试。宿主可以通过 FFI conversation 参数 `allow_insecure_llm_request_headers` 显式放行非本机 HTTP，但这表示宿主已经接受明文传输导致凭据泄露的风险，该风险不属于 runtime core 的责任边界。

## 1.3 Provider 配置

`builtin_models.json` 是用户可见模型/服务商配置的主要来源。运行时通过 `config::build_index_and_resolver()` 建立：

```text
model_uid -> ModelEntry { model_name, provider_uid }
provider_uid -> (api_key, base_url)
```

`providers.rs` 负责静态协议能力：

```rust
pub struct Provider {
    pub prefix: &'static str,
    pub base_url: &'static str,
    pub format: ApiFormat,
    pub tool_choice_style: ToolChoiceStyle,
}
```

## 1.4 错误模型

当前结构化错误在 `error.rs`：

```rust
pub enum ApiError {
    Retryable { status, msg, retry_after, attempts },
    Fatal(FatalError),
    Cancelled,
    LlmFailed(String),
    VlmFailed(String),
    AsrFailed(String),
    OcrFailed(String),
}
```

新代码应优先返回：

- `Retryable`：网络错误、HTTP 408、HTTP 429、HTTP 5xx。
- `Fatal(AuthFailed)`：401/403。
- `Fatal(ModelNotFound)`：404。
- `Fatal(QuotaExceeded)`：402 或可识别为余额/配额不足的 429。
- `Fatal(ContextTooLong)`：上下文超长。
- `Fatal(ContentFiltered)`：安全审核拦截。
- `Cancelled`：用户主动取消。

`LlmFailed/VlmFailed/AsrFailed/OcrFailed` 是兼容旧调用点的粗粒度错误，不适合作为厂商 HTTP 错误的最终形态。

## 1.5 已修正的问题

- `error.rs` 和 `classify.rs` 已清理成 UTF-8 文档与结构化错误实现。
- `classify_http_error()` 会从 OpenAI、Anthropic、DashScope/Baidu 常见字段中提取错误信息。
- `Retry-After` 秒数会进入 `Retryable.retry_after`，供 `retry_with_backoff()` 使用。
- `anthropic_compat.rs` 已从直接 `json()` 改为先检查 HTTP status，并接入统一重试与错误分类。

## 1.6 仍存在的缺陷

这些是代码现状，不是设计目标：

1. `openai_compat.rs` 的文本 LLM 路径错误处理较完整，但 Vision 图片/视频路径仍有旧式 `VlmFailed(String)` 分支，非 2xx 时容易丢失 HTTP status 和 provider 错误类别。
2. `vlm.rs` 的 Qwen native 图片/视频路径直接 `json()`，没有在解析前检查 status；DashScope 401/429/5xx 可能被折叠成解析错误或普通 `VlmFailed`。
3. `asr.rs` 的 multipart 转录路径同样没有统一 HTTP status 分类，ASR 的限流、鉴权和配额错误不会进入 `Retryable/Fatal`。
4. `ocr.rs` 仍是独立错误处理，百度 OCR token 获取和识别调用没有接入统一分类。
5. `nodes/*` 把 `ApiError` 转成 `AIOutput::error(500, "...")`，会丢失 `kind_str()`、`suggestion()`、`retryable_by_user()` 等结构化字段。
6. VLM/ASR/OCR 还没有统一接入 `model_uid/provider_uid` 配置体系，主要依赖环境变量；这和 LLM 文本路径不一致。
7. 流式 SSE 在连接建立前有分类和重试，连接建立后的 `data: {"error": ...}` 仍返回 `LlmFailed(String)`，还没有结构化分类。

## 1.7 建议改造顺序

1. 把所有 HTTP 调用改成：`send -> status check -> classify_http_error -> json parse`。
2. 给 JSON body 可复用的路径接入 `retry_with_backoff()`，包括 Anthropic、OpenAI Vision、Qwen native VLM。
3. 对 multipart ASR/OCR 采用“每次 retry 重新构建 request body”的方式，避免复用已消费的 form/body。
4. 为 `AIOutput::error` 增加结构化 metadata，保留 `ApiError.kind_str()` 和 `suggestion()`。
5. 把 VLM/ASR/OCR 的 key/base_url 也迁移到 `model_uid/provider_uid`，环境变量仅作为兼容 fallback。

## 1.8 与 ai-assistant 的关系

`ai-assistant` 的 `thinking` 会根据 `ApiError` 分类决定：

- 可重试错误：轻量等待后重试。
- fatal 错误：发布 `LLM_ERROR`，进入 `saying` 展示友好提示。
- cancelled：发布 interrupted，进入 `suspended`。

因此 `ai-gateway` 是否正确分类厂商错误，会直接影响前端看到的是“API Key 无效/余额不足/上游限流”，还是泛泛的“LLM 调用失败”。
