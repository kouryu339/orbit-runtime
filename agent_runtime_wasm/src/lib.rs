//! Browser bindings only. Agent and Workflow algorithms live in shared crates.
mod bridge;
mod cluster;
mod events;
mod runtime;

use serde_json::Value;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct BrowserRuntime {
    inner: Arc<runtime::Runtime>,
}

#[wasm_bindgen]
pub async fn create_runtime(
    options_json: String,
    on_event: js_sys::Function,
    on_host: js_sys::Function,
) -> Result<BrowserRuntime, JsValue> {
    bridge::bind(on_event, on_host);
    let options: Value = serde_json::from_str(&options_json).map_err(js_error)?;
    let inner = runtime::Runtime::create(options).await.map_err(js_error)?;
    Ok(BrowserRuntime {
        inner: Arc::new(inner),
    })
}

#[wasm_bindgen]
impl BrowserRuntime {
    pub fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }
    pub fn capabilities(&self) -> String {
        runtime::capabilities().to_string()
    }
    pub async fn start(&self) -> Result<(), JsValue> {
        self.inner.start().await.map_err(js_error)
    }
    pub async fn invoke(&self, request_json: String) -> String {
        let result = match serde_json::from_str::<Value>(&request_json) {
            Ok(request) => self.inner.invoke(request).await,
            Err(error) => {
                serde_json::json!({"schema":"agent-runtime-result/v1","ok":false,"error":{"kind":"invalid_argument","message":error.to_string()}})
            }
        };
        result.to_string()
    }
    pub fn complete_host_request(&self, id: String, response_json: String) -> bool {
        let parsed: Value = match serde_json::from_str(&response_json) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let result = if parsed.get("ok") == Some(&Value::Bool(true)) {
            Ok(parsed.get("value").cloned().unwrap_or(Value::Null))
        } else {
            Err(parsed
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("host operation failed")
                .into())
        };
        self.inner.bridge.complete(&id, result)
    }
    pub fn resource_snapshot(&self) -> Result<String, JsValue> {
        self.inner
            .resource_snapshot()
            .map(|value| value.to_string())
            .map_err(js_error)
    }
    pub fn finish_resource_commit(&self, accepted: bool) -> Result<(), JsValue> {
        self.inner
            .finish_resource_commit(accepted)
            .map_err(js_error)
    }
    pub async fn shutdown(&self) -> Result<(), JsValue> {
        self.inner.shutdown().await.map_err(js_error)
    }
}
fn js_error(error: impl std::fmt::Display) -> JsValue {
    js_sys::Error::new(&error.to_string()).into()
}
