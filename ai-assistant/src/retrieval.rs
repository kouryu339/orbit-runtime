use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use corework::cache::{Cache, CacheExt};
use corework::orchestration::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::keys;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalFailPolicy {
    Soft,
    Hard,
}

impl Default for RetrievalFailPolicy {
    fn default() -> Self {
        Self::Soft
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetrievalConfig {
    pub enabled: bool,
    pub mode: String,
    pub trigger: String,
    pub tool_name: String,
    pub endpoint_id: Option<String>,
    pub profiles: Vec<String>,
    pub top_k: Option<u32>,
    pub score_threshold: Option<f64>,
    pub fail_policy: RetrievalFailPolicy,
    pub inject_as: String,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "before_thinking".to_string(),
            trigger: "first_thinking_per_user_turn".to_string(),
            tool_name: "RagRetrieve".to_string(),
            endpoint_id: None,
            profiles: Vec::new(),
            top_k: Some(5),
            score_threshold: Some(0.3),
            fail_policy: RetrievalFailPolicy::Soft,
            inject_as: "dynamic_context".to_string(),
        }
    }
}

impl RetrievalConfig {
    pub fn before_thinking_enabled(&self) -> bool {
        self.enabled
            && self.mode == "before_thinking"
            && self.inject_as == "dynamic_context"
            && !self.tool_name.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalCacheEntry {
    pub query_hash: String,
    pub turn_id: u64,
    pub tool_name: String,
    pub profiles: Vec<String>,
    pub context: String,
}

pub async fn retrieve_before_thinking(
    ctx: &Context,
    cache: &Arc<dyn Cache>,
    config: &RetrievalConfig,
    query: &str,
    turn_id: u64,
) -> corework::error::Result<Option<String>> {
    let query = query.trim();
    if !config.before_thinking_enabled() || query.is_empty() {
        clear_retrieval_context(cache).await?;
        return Ok(None);
    }

    let query_hash = retrieval_query_hash(config, query);
    if let Some(existing_hash) = cache.get::<String>(keys::RETRIEVAL_QUERY_HASH).await? {
        if existing_hash == query_hash {
            if let Some(context) = cache.get::<String>(keys::RETRIEVAL_CONTEXT).await? {
                if !context.trim().is_empty() {
                    return Ok(Some(context));
                }
            }
        }
    }

    let context = match execute_retrieval_tool(ctx, config, query).await {
        Ok(Some(context)) => context,
        Ok(None) => {
            clear_retrieval_context(cache).await?;
            return Ok(None);
        }
        Err(err) => {
            if config.fail_policy == RetrievalFailPolicy::Hard {
                return Err(err);
            }
            tracing::warn!("pre-thinking retrieval skipped: {}", err);
            clear_retrieval_context(cache).await?;
            return Ok(None);
        }
    };

    let entry = RetrievalCacheEntry {
        query_hash: query_hash.clone(),
        turn_id,
        tool_name: config.tool_name.clone(),
        profiles: config.profiles.clone(),
        context: context.clone(),
    };
    cache.set(keys::RETRIEVAL_CONTEXT, &context, None).await?;
    cache
        .set(keys::RETRIEVAL_QUERY_HASH, &query_hash, None)
        .await?;
    cache.set(keys::RETRIEVAL_TURN_ID, &turn_id, None).await?;
    cache.set(keys::RETRIEVAL_LAST_ENTRY, &entry, None).await?;
    tracing::info!(
        turn_id = turn_id,
        tool_name = %config.tool_name,
        profiles = ?config.profiles,
        context_chars = context.chars().count(),
        "pre-thinking retrieval context cached"
    );

    Ok(Some(context))
}

async fn execute_retrieval_tool(
    ctx: &Context,
    config: &RetrievalConfig,
    query: &str,
) -> corework::error::Result<Option<String>> {
    let executor = ctx.get_dynamic_system(&config.tool_name)?;
    let mut input = HashMap::new();
    input.insert("query".to_string(), Value::String(query.to_string()));
    input.insert(
        "profiles".to_string(),
        Value::Array(
            config
                .profiles
                .iter()
                .map(|profile| Value::String(profile.clone()))
                .collect(),
        ),
    );
    if let Some(top_k) = config.top_k {
        input.insert("top_k".to_string(), Value::Number(top_k.into()));
    }
    if let Some(score_threshold) = config.score_threshold {
        if let Some(number) = serde_json::Number::from_f64(score_threshold) {
            input.insert("score_threshold".to_string(), Value::Number(number));
        }
    }

    let output = executor.execute_dynamic(input, ctx).await?;
    let error_code = output
        .get("error_code")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if error_code != 0 {
        let to_ai = output
            .get("to_ai")
            .and_then(Value::as_str)
            .unwrap_or("retrieval tool failed");
        return Err(corework::error::FrameworkError::SystemError(format!(
            "{} returned error_code {}: {}",
            config.tool_name, error_code, to_ai
        )));
    }

    let to_ai = output
        .get("to_ai")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if to_ai.is_empty() {
        return Ok(None);
    }
    tracing::info!(
        tool_name = %config.tool_name,
        query_chars = query.chars().count(),
        to_ai_chars = to_ai.chars().count(),
        "pre-thinking retrieval tool returned context"
    );

    Ok(Some(format!(
        "## Retrieved Knowledge\n\n{}\n\nUse this context as reference material. If it is irrelevant or insufficient, ignore it and say what still needs confirmation instead of inventing details.",
        to_ai
    )))
}

async fn clear_retrieval_context(cache: &Arc<dyn Cache>) -> corework::error::Result<()> {
    cache.delete(keys::RETRIEVAL_CONTEXT).await?;
    cache.delete(keys::RETRIEVAL_QUERY_HASH).await?;
    cache.delete(keys::RETRIEVAL_TURN_ID).await?;
    cache.delete(keys::RETRIEVAL_LAST_ENTRY).await?;
    Ok(())
}

fn retrieval_query_hash(config: &RetrievalConfig, query: &str) -> String {
    let mut hasher = DefaultHasher::new();
    config.tool_name.hash(&mut hasher);
    config.profiles.hash(&mut hasher);
    config.top_k.hash(&mut hasher);
    config.score_threshold.map(f64::to_bits).hash(&mut hasher);
    query.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
