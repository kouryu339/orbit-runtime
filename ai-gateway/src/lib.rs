#![allow(clippy::too_many_arguments)]

//! 配置表驱动的多提供商 LLM 网关
//!
//! 所有支持 OpenAI 兼容格式的厂商只需在 [`providers`] 配置表加一行，
//!
//! ## 支持的提供商
//!
//! ### 文本对话（LLM）
//! - DeepSeek（`deepseek-*`，DEEPSEEK_API_KEY）
//! - 通义千问（`qwen-*` / `qwen3*`，DASHSCOPE_API_KEY）
//! - 豆包（`doubao-*`，ARK_API_KEY）
//! - MiniMax（`MiniMax-*`，MINIMAX_API_KEY）
//! - 讯飞星火（`spark-*`，SPARK_API_KEY）
//! - 月之暗面·Kimi（`moonshot-*`，MOONSHOT_API_KEY）
//! - 百川智能（`Baichuan*`，BAICHUAN_API_KEY）
//! - 智谱·GLM（`glm-*`，ZHIPU_API_KEY）
//! - 零一万物·Yi（`yi-*`，YI_API_KEY）
//! - 阶跃星辰·Step（`step-*`，STEPFUN_API_KEY）
//! - Claude（`claude-*`，CLAUDE_API_KEY）
//! - GPT（`gpt-*`，OPENAI_API_KEY）
//! - Gemini（`gemini-*`，GEMINI_API_KEY）
//! - Ollama（本地 fallback）
//!
//! ### 视觉语言模型（VLM）
//! - 通义千问 VL（`qwen-vl*`，DASHSCOPE_API_KEY）—— 图片 + 原生视频
//! - 豆包视觉（`doubao-vision*`，ARK_API_KEY）—— 图片 + 原生视频
//! - Gemini（`gemini-*`，GEMINI_API_KEY）—— 图片 + 原生视频
//! - MiniMax VL（`MiniMax-VL*`，MINIMAX_API_KEY）—— 仅图片
//! - GPT-4o Vision（`gpt-4o*`，OPENAI_API_KEY）—— 仅图片
//!
//! ### 语音识别（ASR）
//! - 通义千问 Audio（`qwen-audio*`，DASHSCOPE_API_KEY）
//! - 豆包 ASR（`doubao-asr*`，ARK_API_KEY）
//! - OpenAI Whisper（`whisper-*`，OPENAI_API_KEY）
//!
//!
//! ```no_run
//! # async fn example() {
//! use llm_gateway::{ChatMessage, call_llm, call_vlm, call_asr};
//!
//! // 文本对话（豆包）
//! let messages = vec![ChatMessage::user("你好")];
//! call_llm(&messages, Some("doubao-pro-32k"), None, None, None).await;
//!
//! // 图片理解（豆包视觉）
//! call_vlm("image.jpg", "描述这张图片", None, Some("doubao-vision-pro-32k"), None, None).await;
//!
//! // 语音转文字（通义千问）
//! call_asr("audio.mp3", Some("qwen-audio-turbo"), None, None).await;
//! # }
//! ```

pub mod error;
pub mod types;

pub mod anthropic_compat;
pub mod asr;
pub mod classify;
pub mod config;
pub mod diagnostics;
pub mod dispatch;
pub mod key_store;
pub mod ocr;
pub mod openai_compat;
pub mod providers;
pub mod request_context;
pub mod retry;
pub mod vlm;

pub mod nodes;

// 重新导出常用类型
pub use types::{
    AsrResponse, AsrSegment, BBox2D, ChatMessage, FunctionCall, FunctionDefinition, ImageSize,
    LlmResponse, OcrResult, OcrResultItem, TokenUsage, ToolCall, ToolDefinition, VlmResponse,
};

// 重新导出错误类型 + 重试策略
pub use error::{ApiError, FatalError, FatalKind, Result};
pub use retry::RetryPolicy;

// 重新导出配置类型
pub use config::{
    all_models, all_providers, build_index_and_resolver, find_model, find_provider, init_keys,
    load as load_config, load_from_path as load_config_from_path, models_by_provider,
    save as save_config, set_config_dir, ApiParadigm, EnabledModel, LlmConfig, Modal,
    ModelDefinition, ProviderDefinition, UserProviderConfig,
};

// 重新导出纯函数
pub use asr::call_asr;
pub use ocr::call_baidu_ocr;
pub use vlm::{call_qwen_vl, call_video_vlm, call_vlm};

pub use dispatch::{
    call_llm, call_llm_cancellable, call_llm_decide, call_llm_decide_cancellable,
    call_llm_decide_streaming, call_llm_decide_streaming_cancellable, call_llm_json_cancellable,
    call_llm_with_tools, call_llm_with_tools_cancellable, model_supports_tool_choice,
    model_tool_choice_style,
};
