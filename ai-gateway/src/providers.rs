//! 提供商配置表

use crate::config::{self, ApiFormatType, ToolChoiceStyleType};

/// API 格式类型
#[derive(Clone, Copy, PartialEq)]
pub enum ApiFormat {
    /// OpenAI Chat Completions 格式（/chat/completions）
    OpenAI,
    /// Anthropic Messages 格式（/v1/messages）
    Anthropic,
}

/// `tool_choice` 强制调用方式
#[derive(Clone, Copy, PartialEq)]
pub enum ToolChoiceStyle {
    /// 指定函数名强制调用：`{"type":"function","function":{"name":"xxx"}}`
    /// 适用于 OpenAI、DeepSeek、Qwen、Claude 等
    ForceName,
    /// 强制调用任意工具：`"required"`
    /// 适用于 MiniMax 等不支持 function 指定但支持 required 的模型
    Required,
    /// 不使用 tool_choice（走 JSON prompt 模式）
    None,
}

/// 解析后的 LLM provider 信息（从 builtin_models.json 驱动）
pub struct ResolvedProvider {
    /// base URL
    pub base_url: String,
    /// API 格式
    pub format: ApiFormat,
    /// tool_choice 强制调用方式
    pub tool_choice_style: ToolChoiceStyle,
}

impl From<ApiFormatType> for ApiFormat {
    fn from(t: ApiFormatType) -> Self {
        match t {
            ApiFormatType::OpenAI => ApiFormat::OpenAI,
            ApiFormatType::Anthropic => ApiFormat::Anthropic,
        }
    }
}

impl From<ToolChoiceStyleType> for ToolChoiceStyle {
    fn from(t: ToolChoiceStyleType) -> Self {
        match t {
            ToolChoiceStyleType::ForceName => ToolChoiceStyle::ForceName,
            ToolChoiceStyleType::Required => ToolChoiceStyle::Required,
            ToolChoiceStyleType::None => ToolChoiceStyle::None,
        }
    }
}

/// 根据模型名解析 provider 信息（从 builtin_models.json 驱动）
///
/// 匹配逻辑：遍历所有 provider，找 prefix 非空且模型名以该 prefix 开头的；
/// 无匹配时 fallback 到 prefix 为空的 provider（Ollama 本地）。
pub fn resolve(model: &str) -> ResolvedProvider {
    let providers = config::all_providers();

    // 找 prefix 非空且匹配的
    if let Some(p) = providers
        .iter()
        .filter(|p| !p.prefix.is_empty())
        .find(|p| model.starts_with(&p.prefix))
    {
        return ResolvedProvider {
            base_url: p.default_base_url.clone(),
            format: p.api_format.into(),
            tool_choice_style: p.tool_choice_style.into(),
        };
    }

    // fallback: prefix 为空的 provider
    if let Some(p) = providers.iter().find(|p| p.prefix.is_empty()) {
        return ResolvedProvider {
            base_url: p.default_base_url.clone(),
            format: p.api_format.into(),
            tool_choice_style: p.tool_choice_style.into(),
        };
    }

    // 终极 fallback（不应到达）
    ResolvedProvider {
        base_url: "http://localhost:11434/v1".to_string(),
        format: ApiFormat::OpenAI,
        tool_choice_style: ToolChoiceStyle::None,
    }
}

// ============================================================================
// VLM 提供商配置表
// ============================================================================

/// VLM API 调用风格
pub enum VlmApiStyle {
    /// OpenAI Vision 格式（/chat/completions + image_url content block）
    OpenAIVision,
    /// 通义千问 VL 原生端点（DashScope multimodal-generation）
    QwenNative,
}

/// VLM 提供商静态配置
pub struct VlmProvider {
    pub prefix: &'static str,
    pub base_url: &'static str,
    pub env_key: &'static str,
    pub default_model: &'static str,
    pub api_style: VlmApiStyle,
    /// 是否原生支持视频输入（直接理解视频内容，无需 ffmpeg 提取音轨）
    pub supports_video: bool,
}

pub static VLM_PROVIDERS: &[VlmProvider] = &[
    // 通义千问 VL（原生 DashScope 端点，支持视频）
    VlmProvider {
        prefix: "qwen-vl",
        base_url:
            "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation",
        env_key: "DASHSCOPE_API_KEY",
        default_model: "qwen-vl-max-latest",
        api_style: VlmApiStyle::QwenNative,
        supports_video: true,
    },
    // 豆包视觉（火山引擎，OpenAI Vision 格式，支持视频 URL）
    VlmProvider {
        prefix: "doubao-vision",
        base_url: "https://ark.cn-beijing.volces.com/api/v3",
        env_key: "ARK_API_KEY",
        default_model: "doubao-vision-pro-32k",
        api_style: VlmApiStyle::OpenAIVision,
        supports_video: true,
    },
    // MiniMax VL（仅图片）
    VlmProvider {
        prefix: "MiniMax-VL",
        base_url: "https://api.minimaxi.com/v1",
        env_key: "MINIMAX_API_KEY",
        default_model: "MiniMax-VL-01",
        api_style: VlmApiStyle::OpenAIVision,
        supports_video: false,
    },
    // GPT-4o（仅图片，不支持视频文件输入）
    VlmProvider {
        prefix: "gpt-4o",
        base_url: "https://api.openai.com/v1",
        env_key: "OPENAI_API_KEY",
        default_model: "gpt-4o",
        api_style: VlmApiStyle::OpenAIVision,
        supports_video: false,
    },
    // Gemini（支持视频文件，OpenAI 兼容端点）
    VlmProvider {
        prefix: "gemini-",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        env_key: "GEMINI_API_KEY",
        default_model: "gemini-2.0-flash",
        api_style: VlmApiStyle::OpenAIVision,
        supports_video: true,
    },
    // fallback: 通义千问多模态（性能最强视觉理解模型）
    VlmProvider {
        prefix: "",
        base_url:
            "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation",
        env_key: "DASHSCOPE_API_KEY",
        default_model: "qwen3.5-plus",
        api_style: VlmApiStyle::QwenNative,
        supports_video: true,
    },
];

pub fn resolve_vlm(model: &str) -> &'static VlmProvider {
    VLM_PROVIDERS
        .iter()
        .filter(|p| !p.prefix.is_empty())
        .find(|p| model.starts_with(p.prefix))
        .unwrap_or(&VLM_PROVIDERS[VLM_PROVIDERS.len() - 1])
}

/// 解析视频理解提供商：
/// - 若 model 非空且该提供商支持视频，直接使用
/// - 若 model 非空但不支持视频，返回 Err（让调用方决定是否 fallback 到 ASR 路线）
/// - 若 model 为空，选第一个支持视频的提供商（默认 qwen-vl-max-latest）
pub fn resolve_vlm_for_video(model: &str) -> Result<&'static VlmProvider, &'static str> {
    if model.is_empty() {
        // 找第一个支持视频的
        VLM_PROVIDERS
            .iter()
            .find(|p| p.supports_video)
            .ok_or("没有可用的视频理解提供商")
    } else {
        let provider = VLM_PROVIDERS
            .iter()
            .filter(|p| !p.prefix.is_empty())
            .find(|p| model.starts_with(p.prefix))
            .unwrap_or(&VLM_PROVIDERS[VLM_PROVIDERS.len() - 1]);
        if provider.supports_video {
            Ok(provider)
        } else {
            Err("该模型不支持原生视频输入，请选择 qwen-vl-max-latest / doubao-vision-pro-32k / gemini-2.0-flash")
        }
    }
}

// ============================================================================
// ASR 提供商配置表
// ============================================================================

/// ASR API 调用风格
pub enum AsrApiStyle {
    /// 阿里 DashScope qwen-audio（compatible-mode audio/transcriptions）
    QwenAudio,
    /// OpenAI Whisper 兼容格式（multipart/form-data）
    WhisperCompat,
}

/// ASR 提供商静态配置
pub struct AsrProvider {
    pub prefix: &'static str,
    pub base_url: &'static str,
    pub env_key: &'static str,
    pub default_model: &'static str,
    pub api_style: AsrApiStyle,
}

pub static ASR_PROVIDERS: &[AsrProvider] = &[
    // 阿里 qwen-audio
    AsrProvider {
        prefix: "qwen-audio",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        env_key: "DASHSCOPE_API_KEY",
        default_model: "qwen-audio-turbo",
        api_style: AsrApiStyle::QwenAudio,
    },
    // 豆包 ASR（火山引擎，Whisper 兼容格式）
    AsrProvider {
        prefix: "doubao-asr",
        base_url: "https://ark.cn-beijing.volces.com/api/v3",
        env_key: "ARK_API_KEY",
        default_model: "doubao-asr",
        api_style: AsrApiStyle::WhisperCompat,
    },
    // OpenAI Whisper
    AsrProvider {
        prefix: "whisper-",
        base_url: "https://api.openai.com/v1",
        env_key: "OPENAI_API_KEY",
        default_model: "whisper-1",
        api_style: AsrApiStyle::WhisperCompat,
    },
    // fallback: qwen-audio-turbo
    AsrProvider {
        prefix: "",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        env_key: "DASHSCOPE_API_KEY",
        default_model: "qwen-audio-turbo",
        api_style: AsrApiStyle::QwenAudio,
    },
];

pub fn resolve_asr(model: &str) -> &'static AsrProvider {
    ASR_PROVIDERS
        .iter()
        .filter(|p| !p.prefix.is_empty())
        .find(|p| model.starts_with(p.prefix))
        .unwrap_or(&ASR_PROVIDERS[ASR_PROVIDERS.len() - 1])
}
