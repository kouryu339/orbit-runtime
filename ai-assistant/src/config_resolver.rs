//! Conversation-scoped 配置解析。
//! 协议见 `docs/AGENT_GATEWAY_ADMISSION.md` §5。本模块封装"读链"，
//! 让 thinking / compact 等热路径只调用一个函数。
//! ## 模型来源（优先级从高到低）
//! - 默认 Agent：Conversation 模型 → Runtime 全局模型。
//! - 子/处理 Agent：Agent 模型 → Conversation 模型 → Runtime 全局模型。
//! - 后台 Agent 在创建时先把任务参数 `model_id` 或 profile 默认模型写入 Agent
//!   模型槽，因此自然遵循：任务参数 → profile 默认 → Conversation → Runtime 全局。
//!    只接受显式选择的当前模型；不会从内置模型目录或索引中自动兜底。
//! 写入路径：`Conversation::set_model` / `AIAssistant::set_model` 都只写入
//! conversation 层，从而避免多会话并行下的"全局变更被静默传染"。

use std::sync::Arc;

use corework::cache::{Cache, CacheExt};

use crate::context::keys;

/// Conversation 层配置 key（写入 `Conversation::ledger().cache()`）。
pub mod conversation_keys {
    /// 当前会话使用的推理模型名 —— `String`
    pub const CONFIG_MODEL: &str = "config:model";
    /// 当前会话使用的摘要模型名 —— `String`
    /// 缺省时回退到 `CONFIG_MODEL`，再回退到推理路径解析结果。
    pub const CONFIG_SUMMARY_MODEL: &str = "config:summary_model";
    /// 当前会话使用的语言 —— `String`
    pub const CONFIG_LANGUAGE: &str = "config:language";
}

/// 拿到 Conversation 共享的 ledger cache（如果当前线程有全局 conversation 上下文）。
fn conversation_cache() -> Option<Arc<dyn Cache>> {
    crate::conversation::Conversation::global().map(|c| c.ledger().cache())
}

async fn registered_model_from_cache(cache: &Arc<dyn Cache>, key: &str) -> Option<u32> {
    let name = cache
        .get::<String>(key)
        .await
        .ok()
        .flatten()
        .filter(|name| !name.trim().is_empty())?;
    match llm_gateway::key_store::find_by_name(&name) {
        Some(uid) => Some(uid),
        None => {
            tracing::warn!(
                cache_key = key,
                model = %name,
                "configured model is not registered; continuing model fallback"
            );
            None
        }
    }
}

/// 按 Agent 身份解析推理模型的 `model_uid`。
///
/// `conversation_cache` 必须是 cluster 默认 Agent 的 cache。后台任务的显式
/// `model_id` 和 profile 默认模型都在创建时归一化到其 `agent_cache.keys::MODEL`。
pub async fn resolve_inference_model_uid_for_agent(
    agent_cache: &Arc<dyn Cache>,
    conversation_cache: Option<&Arc<dyn Cache>>,
    is_default_agent: bool,
) -> Option<u32> {
    if !is_default_agent {
        if let Some(uid) = registered_model_from_cache(agent_cache, keys::MODEL).await {
            return Some(uid);
        }
    }

    let conversation_cache = conversation_cache.unwrap_or(agent_cache);
    if let Some(uid) =
        registered_model_from_cache(conversation_cache, conversation_keys::CONFIG_MODEL).await
    {
        return Some(uid);
    }

    llm_gateway::key_store::current()
}

/// Default-Agent compatibility wrapper. Runtime routing code should prefer
/// `resolve_inference_model_uid_for_agent` and pass the verified Agent scope.
pub async fn resolve_inference_model_uid(agent_cache: &Arc<dyn Cache>) -> Option<u32> {
    resolve_inference_model_uid_for_agent(agent_cache, Some(agent_cache), true).await
}

/// 解析摘要模型的 `model_uid`，缺省时回退到推理模型 `fallback_uid`。
pub async fn resolve_summary_model_uid(agent_cache: &Arc<dyn Cache>, fallback_uid: u32) -> u32 {
    if let Ok(Some(name)) = agent_cache
        .get::<String>(conversation_keys::CONFIG_SUMMARY_MODEL)
        .await
    {
        if let Some(uid) = llm_gateway::key_store::find_by_name(&name) {
            return uid;
        } else {
            tracing::warn!(
                "scoped config:summary_model='{}' 未在 key_store 中找到，回退到推理模型",
                name
            );
        }
    }
    fallback_uid
}

/// 写入当前 conversation 层配置。
/// `Conversation::global()` 缺失时不猜测 Agent 作用域，也不修改 Runtime 全局模型。
pub async fn write_conversation_model(model_name: &str) -> crate::Result<()> {
    if let Some(cache) = conversation_cache() {
        cache
            .set(
                conversation_keys::CONFIG_MODEL,
                &model_name.to_string(),
                None,
            )
            .await?;
    }
    Ok(())
}

pub async fn write_conversation_summary_model(model_name: &str) -> crate::Result<()> {
    if let Some(cache) = conversation_cache() {
        cache
            .set(
                conversation_keys::CONFIG_SUMMARY_MODEL,
                &model_name.to_string(),
                None,
            )
            .await?;
    }
    Ok(())
}

pub async fn write_conversation_language(language: &str) -> crate::Result<()> {
    if let Some(cache) = conversation_cache() {
        cache
            .set(
                conversation_keys::CONFIG_LANGUAGE,
                &language.to_string(),
                None,
            )
            .await?;
    }
    Ok(())
}
