use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use corework::rpc_tool::RuntimeToolMetadata;

static RUNTIME_TOOLS: OnceLock<RwLock<HashMap<String, RuntimeToolMetadata>>> = OnceLock::new();

fn registry() -> &'static RwLock<HashMap<String, RuntimeToolMetadata>> {
    RUNTIME_TOOLS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn register_runtime_tool(metadata: RuntimeToolMetadata) {
    let mut guard = registry().write().expect("runtime tool registry poisoned");
    guard.insert(metadata.name.clone(), metadata);
}

pub fn get_runtime_tool(name: &str) -> Option<RuntimeToolMetadata> {
    let guard = registry().read().expect("runtime tool registry poisoned");
    guard.get(name).cloned()
}

pub fn list_runtime_tool_names() -> Vec<String> {
    let guard = registry().read().expect("runtime tool registry poisoned");
    guard.keys().cloned().collect()
}
