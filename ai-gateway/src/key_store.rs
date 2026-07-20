//! 内存索引 + API Key 解析器（供 dispatch 使用）
//!
//! ## 数据流
//!
//! sunwoo 启动时注册解析器:
//!   set_resolver(|provider_uid: u32| -> Option<(api_key, base_url)>)
//!
//! 用户保存配置时写入索引:
//!   model_uid → ModelEntry { model_name, provider_uid }
//!
//! dispatch 时:
//!   model_uid → model_name + provider_uid
//!   → providers::resolve(model_name) → 静态 format / default_base_url
//!   → resolver(provider_uid)         → 用户的 api_key + base_url

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

/// 单个 model_uid 对应的信息
#[derive(Clone, Debug)]
pub struct ModelEntry {
    /// API 调用名，如 "qwen-plus"
    pub model_name: String,
    /// 厂商实例唯一 id（与 UserProviderConfig.id 对应）
    pub provider_uid: u32,
    /// 有效上下文窗口大小（tokens）
    ///
    /// 取 builtin_models.json 中 contextWindow 与用户 maxContextTokens 的较小值，
    pub context_window: u32,
}

/// provider_uid → runtime provider config
#[derive(Clone, Debug)]
pub struct ProviderRuntimeConfig {
    pub api_key: String,
    pub base_url: String,
    pub api_paradigm: Option<crate::config::ApiParadigm>,
    pub prompt_cache_control: bool,
}

/// provider_uid → provider runtime config
type Resolver = Arc<dyn Fn(u32) -> Option<ProviderRuntimeConfig> + Send + Sync>;

static MODEL_INDEX: Lazy<RwLock<HashMap<u32, ModelEntry>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

static RESOLVER: Lazy<RwLock<Option<Resolver>>> = Lazy::new(|| RwLock::new(None));

/// 全局当前选中的 model_uid（0 = 未选择）
static CURRENT_MODEL_UID: AtomicU32 = AtomicU32::new(0);

/// 注册 api_key 解析器
pub fn set_resolver<F>(f: F)
where
    F: Fn(u32) -> Option<ProviderRuntimeConfig> + Send + Sync + 'static,
{
    *RESOLVER.write().unwrap() = Some(Arc::new(f));
}

/// 按 model_uid 查询
pub fn get(model_uid: u32) -> Option<ModelEntry> {
    MODEL_INDEX.read().unwrap().get(&model_uid).cloned()
}

/// 通过 provider_uid 解析 api_key + base_url
pub fn resolve_provider(provider_uid: u32) -> Option<(String, String)> {
    resolve_provider_runtime(provider_uid).map(|p| (p.api_key, p.base_url))
}

/// 通过 provider_uid 解析完整 provider runtime config
pub fn resolve_provider_runtime(provider_uid: u32) -> Option<ProviderRuntimeConfig> {
    let guard = RESOLVER.read().unwrap();
    guard.as_ref().and_then(|f| f(provider_uid))
}

/// 清空 model 索引
pub fn clear() {
    MODEL_INDEX.write().unwrap().clear();
}

/// 按模型名反向查找 model_uid（第一个匹配）
pub fn find_by_name(model_name: &str) -> Option<u32> {
    MODEL_INDEX
        .read()
        .unwrap()
        .iter()
        .find(|(_, e)| e.model_name == model_name)
        .map(|(uid, _)| *uid)
}

/// 写入完整 model 索引（保存配置时调用）
pub fn reload(entries: impl Iterator<Item = (u32, ModelEntry)>) {
    let mut map = MODEL_INDEX.write().unwrap();
    map.clear();
    for (model_uid, entry) in entries {
        map.insert(model_uid, entry);
    }
}

/// 设置全局当前选中模型
pub fn set_current(model_uid: u32) {
    CURRENT_MODEL_UID.store(model_uid, Ordering::Relaxed);
}

/// 获取全局当前选中的 model_uid（0 = 未选择）
pub fn current() -> Option<u32> {
    let uid = CURRENT_MODEL_UID.load(Ordering::Relaxed);
    if uid == 0 {
        None
    } else {
        Some(uid)
    }
}

/// 获取全局当前选中的 model_uid，fallback 到索引中第一个可用模型
pub fn current_or_first() -> Option<u32> {
    if let Some(uid) = current() {
        return Some(uid);
    }
    MODEL_INDEX.read().unwrap().keys().copied().next()
}
