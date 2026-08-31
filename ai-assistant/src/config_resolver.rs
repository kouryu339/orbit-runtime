//! Conversation-scoped 配置解析。
//! 协议见 `docs/AGENT_GATEWAY_ADMISSION.md` §5。本模块封装"读链"，
//! 让 thinking / compact 等热路径只调用一个函数。
//! ## 三层来源（优先级从高到低）
//! 1. **Conversation 层**：`Conversation::global().ledger().cache()`
//!    存储在 conversation 共享 ExecutionUnit cache 上的配置（多 agent 共享）。
//! 2. **Agent 层**：传入的 `agent_cache.get(keys::MODEL / keys::LANGUAGE)`
//!    保留兼容老路径（持久化 / 子 agent 默认值）。
//! 3. **运行时全局**：`llm_gateway::key_store::current()`
//!    只接受显式选择的当前模型；不会从内置模型目录或索引中自动兜底。
//! 写入路径：`AIAssistant::set_model` / 未来 `Conversation::set_summary_model`
//! 都写到 conversation 层，从而避免多会话并行下的"全局变更被静默传染"。

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

/// 解析推理模型的 `model_uid`。
/// 顺序：conversation cache `config:model` → agent cache `keys::MODEL` →
/// runtime 全局 `key_store::current()`。
/// `agent_cache` 通常为 `agent.sm.unit().cache()` 或 thinking on_enter 内
/// 拿到的 scoped cache。返回 `None` 表示无任何可用模型。
pub async fn resolve_inference_model_uid(agent_cache: &Arc<dyn Cache>) -> Option<u32> {
    let scoped_name = agent_cache
        .get::<String>(conversation_keys::CONFIG_MODEL)
        .await
        .ok()
        .flatten()
        .filter(|name| !name.is_empty());
    let agent_name = match scoped_name {
        Some(name) => Some(name),
        None => agent_cache
            .get::<String>(keys::MODEL)
            .await
            .ok()
            .flatten()
            .filter(|name| !name.is_empty()),
    };
    if let Some(name) = agent_name {
        if let Some(uid) = llm_gateway::key_store::find_by_name(&name) {
            return Some(uid);
        } else {
            tracing::warn!(
                "scoped config:model='{}' 未在 key_store 中找到，且不会回退到未显式选择的模型",
                name
            );
        }
    }
    llm_gateway::key_store::current()
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

/// 写入 conversation 层配置。返回写入成功与否。
/// `Conversation::global()` 缺失时（极少见，单元测试场景）写入 agent 层兜底。
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
