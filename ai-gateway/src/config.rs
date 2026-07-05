//! LLM 配置文件管理（JSON 格式）
//!
//! 内置定义从 `config/builtin_models.json` 加载（随 crate 打包）
//! 用户配置路径：`%APPDATA%/sunwoo/llm_config.json`
//!
//! ## 用户配置结构
//!
//! ```json
//! {
//!   "providers": [
//!     {
//!       "id": 1,
//!       "name": "通义千问",
//!       "type": "qwen",
//!       "apiKey": "sk-xxx",
//!       "baseUrl": "",
//!       "enabledModels": [
//!         { "uid": 1, "modelId": "qwen-plus" },
//!         { "uid": 2, "modelId": "qwen-max" }
//!       ]
//!     }
//!   ]
//! }
//! ```
//!
//! ## 路由流程
//!
//! ```text
//! call_llm(model_uid=1)
//!   → key_store.get(1)          → { model_name: "qwen-plus", provider_id: 1 }
//!   → key_store.resolve(1)      → (api_key: "sk-xxx", base_url: "")
//!   → providers::resolve("qwen-plus") → 静态 format / default_base_url
//!   → 发起 API 请求
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

// ============================================================================
// 模态枚举
// ============================================================================

/// 输入/输出模态类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modal {
    /// 文字
    Text,
    /// 图片
    Image,
    /// 视频
    Video,
    /// 音频
    Audio,
}

impl Modal {
    pub fn as_str(&self) -> &'static str {
        match self {
            Modal::Text => "text",
            Modal::Image => "image",
            Modal::Video => "video",
            Modal::Audio => "audio",
        }
    }
}

impl std::fmt::Display for Modal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// 内置定义（从 builtin_models.json 加载）
// ============================================================================

/// 内置 JSON 根对象
#[derive(Debug, Clone, Deserialize)]
pub struct BuiltinSchema {
    pub providers: Vec<ProviderDefinition>,
    pub models: Vec<ModelDefinition>,
}

/// 模型定义
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDefinition {
    /// 模型标识符，如 "qwen-plus"
    pub id: String,
    /// 模型显示名称，如 "Qwen Plus"
    pub name: String,
    /// 开发厂商，如 "阿里云"、"DeepSeek"、"OpenAI"
    pub developer: String,
    /// 模型上下文窗口大小（tokens），None 表示未知/不限
    /// 来自 builtin_models.json 中的 contextWindow 字段
    #[serde(alias = "contextWindow", default)]
    pub context_window: Option<u32>,
    /// 输入模态
    #[serde(alias = "inputModal")]
    pub input_modal: Vec<Modal>,
    /// 输出模态
    #[serde(alias = "outputModal")]
    pub output_modal: Vec<Modal>,
    #[serde(alias = "providerPrefix")]
    pub provider_prefix: String,
    /// 是否为该类型默认模型
    #[serde(default)]
    pub default: bool,
}

/// 厂商定义
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDefinition {
    /// 厂商唯一标识，如 "qwen"
    pub id: String,
    /// 厂商显示名称
    pub name: String,
    pub prefix: String,
    /// 默认 base URL
    #[serde(alias = "defaultBaseUrl")]
    pub default_base_url: String,
    /// 支持的 API 格式
    #[serde(alias = "apiFormat")]
    pub api_format: ApiFormatType,
    /// tool_choice 风格
    #[serde(alias = "toolChoiceStyle")]
    pub tool_choice_style: ToolChoiceStyleType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiFormatType {
    OpenAI,
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoiceStyleType {
    ForceName,
    Required,
    None,
}

/// Upstream provider HTTP/API calling paradigm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiParadigm {
    /// Anthropic Messages API, usually `/v1/messages`.
    #[serde(rename = "anthropic_messages")]
    AnthropicMessages,
    /// OpenAI Chat Completions API, usually `/v1/chat/completions`.
    #[serde(rename = "openai_chat_completions")]
    OpenAiChatCompletions,
    /// OpenAI Responses API, usually `/v1/responses`.
    #[serde(rename = "openai_responses")]
    OpenAiResponses,
}

// ============================================================================
// 内置数据（静态加载）
// ============================================================================

/// 从紧邻 crate 的 config/builtin_models.json 加载内置定义
///
/// 构建机的 `CARGO_MANIFEST_DIR` 路径（这在跨平台部署/容器中会失效）。
fn load_builtin_schema() -> BuiltinSchema {
    const BUILTIN_JSON: &str = include_str!("../config/builtin_models.json");
    serde_json::from_str(BUILTIN_JSON)
        .unwrap_or_else(|e| panic!("解析 builtin_models.json 失败: {}", e))
}

lazy_static::lazy_static! {
    static ref BUILTIN_SCHEMA: BuiltinSchema = load_builtin_schema();
}

// ============================================================================
// 用户配置结构（uid 体系）
// ============================================================================

/// 单个已启用的模型条目
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnabledModel {
    /// 全局唯一 u32，dispatch 路由键，一旦分配不变
    pub uid: u32,
    /// 对应 builtin_models.json 中的模型 id（如 "qwen-plus"）
    #[serde(alias = "modelId", alias = "model_id")]
    pub model_id: String,
    ///
    /// 规则：不能超过 builtin_models.json 中该模型的 contextWindow。
    /// None = 使用模型 contextWindow × 0.75（或默认 60000）。
    /// 典型用途：用户觉得用不到全量 context，可以设小一些节省费用。
    #[serde(alias = "maxContextTokens", alias = "max_context_tokens", default)]
    pub max_context_tokens: Option<u32>,
}

/// 用户配置的厂商实例
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserProviderConfig {
    /// 唯一 u32，自动分配，resolve 路由键
    pub id: u32,
    /// 用户自定义显示名称（如 "通义千问"、"Claude 代理A"）
    pub name: String,
    /// builtin_models.json 中的厂商类型 id（如 "claude"、"qwen"），
    /// 仅用于查询静态定义（api format、default base url 等）
    #[serde(rename = "type")]
    pub builtin_type: String,
    /// API Key
    #[serde(alias = "apiKey", alias = "api_key")]
    pub api_key: String,
    /// 自定义 base_url，空串则使用 builtin_models.json 中的默认值
    #[serde(alias = "baseUrl", alias = "base_url", default)]
    pub base_url: String,
    /// 上游 API 调用范式。None 表示按 builtin provider 定义推断。
    #[serde(alias = "apiParadigm", alias = "api_paradigm", default)]
    pub api_paradigm: Option<ApiParadigm>,
    #[serde(alias = "promptCacheControl", alias = "prompt_cache_control", default)]
    pub prompt_cache_control: bool,
    /// 该厂商实例下已启用的模型
    #[serde(alias = "enabledModels", alias = "enabled_models", default)]
    pub enabled_models: Vec<EnabledModel>,
}

/// 完整用户配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LlmConfig {
    #[serde(default)]
    pub providers: Vec<UserProviderConfig>,
    #[serde(
        alias = "currentModelUid",
        alias = "current_model_uid",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub current_model_uid: Option<u32>,
}

// ============================================================================
// 配置路径
// ============================================================================

/// 注入的配置目录（由 Tauri 在 setup() 中调用 set_config_dir 设置）
static CONFIG_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// 设置配置文件的存储目录（应在 app 启动时调用一次，优先于默认路径）
///
/// 典型用法：在 Tauri `setup()` 中传入 `app.path().app_local_data_dir()`，
/// 使每个项目的 llm_config.json 存储在各自独立的目录中，互不干扰。
pub fn set_config_dir(dir: PathBuf) {
    CONFIG_DIR_OVERRIDE.set(dir).ok();
}

fn config_dir() -> PathBuf {
    if let Some(dir) = CONFIG_DIR_OVERRIDE.get() {
        return dir.clone();
    }
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("sunwoo")
}

fn config_path() -> PathBuf {
    config_dir().join("llm_config.json")
}

// ============================================================================
// 配置加载/保存 + key_store 初始化
// ============================================================================

/// 加载用户配置，不存在时返回默认空配置
pub fn load() -> Result<LlmConfig, String> {
    load_from_path(config_path())
}

pub fn load_from_path(path: impl AsRef<Path>) -> Result<LlmConfig, String> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(LlmConfig::default());
    }
    let s = std::fs::read_to_string(path).map_err(|e| format!("读取 llm_config.json 失败: {e}"))?;
    serde_json::from_str(&s).map_err(|e| format!("解析 llm_config.json 失败: {e}"))
}

/// 建立 key_store 索引并注册 resolver
///
/// - 索引：model_uid → { model_name, provider_id(u32), context_window }
/// - resolver：provider_id(u32) → (api_key, base_url)
pub fn build_index_and_resolver(config: LlmConfig) {
    // model_uid → ModelEntry
    let entries: Vec<(u32, crate::key_store::ModelEntry)> = config
        .providers
        .iter()
        .flat_map(|p| {
            p.enabled_models.iter().map(move |em| {
                // 从 builtin 查模型的 context_window 上限
                let builtin_cw = find_model(&em.model_id)
                    .and_then(|m| m.context_window)
                    .unwrap_or(8192);

                // 取用户自定义值与 builtin 上限的较小值
                let effective_cw = match em.max_context_tokens {
                    Some(user_cw) => user_cw.min(builtin_cw),
                    // 未配置：取 builtin 的 75%，最低 4096
                    None => ((builtin_cw as f64 * 0.75) as u32).max(4096),
                };

                (
                    em.uid,
                    crate::key_store::ModelEntry {
                        model_name: em.model_id.clone(),
                        provider_uid: p.id,
                        context_window: effective_cw,
                    },
                )
            })
        })
        .collect();

    crate::key_store::reload(entries.into_iter());
    if let Some(uid) = config.current_model_uid {
        crate::key_store::set_current(uid);
    }

    // provider_id(u32) → (api_key, base_url)
    let config_arc = Arc::new(config);
    crate::key_store::set_resolver(move |provider_id: u32| {
        config_arc
            .providers
            .iter()
            .find(|p| p.id == provider_id)
            .map(|p| crate::key_store::ProviderRuntimeConfig {
                api_key: p.api_key.clone(),
                base_url: p.base_url.clone(),
                api_paradigm: p.api_paradigm,
                prompt_cache_control: p.prompt_cache_control,
            })
    });
}

/// app 启动时调用：加载配置并建立内存索引
pub fn init_keys() -> Result<(), String> {
    let config = load()?;
    build_index_and_resolver(config);
    Ok(())
}

/// 保存用户配置（持久化 + 重建内存索引）
pub fn save(config: &LlmConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let s = serde_json::to_string_pretty(config)
        .map_err(|e| format!("序列化 llm_config.json 失败: {e}"))?;
    std::fs::write(&path, s).map_err(|e| format!("写入 llm_config.json 失败: {e}"))?;
    build_index_and_resolver(config.clone());
    Ok(())
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 根据模型 ID 查找模型定义
pub fn find_model(model_id: &str) -> Option<&'static ModelDefinition> {
    BUILTIN_SCHEMA.models.iter().find(|m| m.id == model_id)
}

/// 根据厂商 ID 查找厂商定义
pub fn find_provider(provider_id: &str) -> Option<&'static ProviderDefinition> {
    BUILTIN_SCHEMA
        .providers
        .iter()
        .find(|p| p.id == provider_id)
}

/// 获取所有模型定义
pub fn all_models() -> &'static [ModelDefinition] {
    &BUILTIN_SCHEMA.models
}

/// 获取所有厂商定义
pub fn all_providers() -> &'static [ProviderDefinition] {
    &BUILTIN_SCHEMA.providers
}

/// 获取某厂商的所有模型
pub fn models_by_provider(provider_id: &str) -> Vec<&'static ModelDefinition> {
    let provider = match find_provider(provider_id) {
        Some(p) => p,
        None => return vec![],
    };
    BUILTIN_SCHEMA
        .models
        .iter()
        .filter(|m| m.provider_prefix == provider.prefix)
        .collect()
}
