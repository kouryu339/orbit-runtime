use corework::{
    ai_system::AIOutput,
    error::{FrameworkError, Result},
    orchestration::Context,
    rpc_tool::{
        validate_runtime_tool_for_endpoint, AgentToolRequest, RpcEndpointInfo, RpcToolClient,
        RuntimeToolMetadata,
    },
};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::oneshot;
use wasm_bindgen::JsValue;

thread_local! {
    static EVENT: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
    static HOST: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
}
pub fn bind(event: js_sys::Function, host: js_sys::Function) {
    EVENT.with(|slot| *slot.borrow_mut() = Some(event));
    HOST.with(|slot| *slot.borrow_mut() = Some(host));
}
pub fn emit(value: &Value) {
    EVENT.with(|slot| {
        if let Some(callback) = slot.borrow().as_ref() {
            let _ = callback.call1(&JsValue::NULL, &JsValue::from_str(&value.to_string()));
        }
    });
}
fn host(value: &Value) -> Result<()> {
    HOST.with(|slot| {
        slot.borrow()
            .as_ref()
            .ok_or_else(|| FrameworkError::InvalidOperation("host adapter is unavailable".into()))?
            .call1(&JsValue::NULL, &JsValue::from_str(&value.to_string()))
            .map(|_| ())
            .map_err(|_| FrameworkError::SystemError("host adapter dispatch failed".into()))
    })
}

#[derive(Default)]
pub struct HostBridge {
    next: AtomicU64,
    pending: Mutex<HashMap<String, oneshot::Sender<std::result::Result<Value, String>>>>,
}
struct PendingRequest {
    bridge: Arc<HostBridge>,
    id: String,
}
impl Drop for PendingRequest {
    fn drop(&mut self) {
        if self
            .bridge
            .pending
            .lock()
            .unwrap()
            .remove(&self.id)
            .is_some()
        {
            let _ = host(&json!({"type":"host.cancel","id":self.id}));
        }
    }
}
impl HostBridge {
    pub async fn request(
        self: &Arc<Self>,
        operation: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Value> {
        let id = format!("host-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.len() >= 256 {
                return Err(FrameworkError::InvalidOperation(
                    "too many pending host operations".into(),
                ));
            }
            pending.insert(id.clone(), sender);
        }
        let _reservation = PendingRequest {
            bridge: self.clone(),
            id: id.clone(),
        };
        host(
            &json!({"type":"host.request","id":id,"operation":operation,"payload":payload,"timeout_ms":timeout_ms}),
        )?;
        corework::platform::time::timeout(std::time::Duration::from_millis(timeout_ms), receiver)
            .await
            .map_err(|_| FrameworkError::SystemError("host operation timed out".into()))?
            .map_err(|_| FrameworkError::InvalidOperation("host operation cancelled".into()))?
            .map_err(FrameworkError::SystemError)
    }
    pub fn complete(&self, id: &str, response: std::result::Result<Value, String>) -> bool {
        self.pending
            .lock()
            .unwrap()
            .remove(id)
            .is_some_and(|sender| sender.send(response).is_ok())
    }
    pub fn cancel_all(&self) {
        let pending = std::mem::take(&mut *self.pending.lock().unwrap());
        for (id, sender) in pending {
            let _ = sender.send(Err("runtime closed".into()));
            let _ = host(&json!({"type":"host.cancel","id":id}));
        }
    }
}

pub struct HostToolClient {
    pub bridge: Arc<HostBridge>,
}
#[async_trait::async_trait]
impl RpcToolClient for HostToolClient {
    async fn execute(
        &self,
        endpoint: RpcEndpointInfo,
        metadata: RuntimeToolMetadata,
        request: AgentToolRequest,
        _: &Context,
    ) -> Result<AIOutput> {
        validate_runtime_tool_for_endpoint(&endpoint, &metadata)?;
        let value = self
            .bridge
            .request(
                "rpc.execute",
                json!({"endpoint_id":endpoint.endpoint_id,"request":request,"metadata":metadata}),
                endpoint.timeout_ms,
            )
            .await?;
        let output: AIOutput = serde_json::from_value(value)?;
        if output.to_ai.trim().is_empty() {
            return Err(FrameworkError::InvalidData(
                "host tool result requires nonempty to_ai".into(),
            ));
        }
        Ok(output)
    }
}
