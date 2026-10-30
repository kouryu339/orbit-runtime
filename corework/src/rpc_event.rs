//! Host-owned event ingestion. No tool registry, task scheduling or filesystem IO.
use crate::rpc_proto::events_v1::{
    self as wire, event_frame::Message,
    event_source_service_client::EventSourceServiceClient as Client,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{watch, Notify};
use tonic::{transport::Channel, Code};

pub const COMMANDS: &[&str] = &[
    "runtime.add_event_source",
    "runtime.get_event_source_definitions",
    "runtime.subscribe_events",
    "runtime.poll_source_events",
    "runtime.acknowledge_events",
    "runtime.unsubscribe_events",
];
const SCHEMA: &str = "corework-event-source/v1";
const MAX_BYTES: usize = 65536;
#[derive(Debug)]
pub struct EventError {
    pub kind: &'static str,
    pub message: String,
}
type Result<T> = std::result::Result<T, EventError>;
fn error(kind: &'static str, message: impl Into<String>) -> EventError {
    EventError {
        kind,
        message: message.into(),
    }
}
fn invalid(message: impl Into<String>) -> EventError {
    error("invalid_argument", message)
}
fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| invalid(e.to_string()))
}
fn id(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 128
}
fn rpc_error(s: tonic::Status) -> EventError {
    let kind = match s.code() {
        Code::OutOfRange => "cursor_expired",
        Code::FailedPrecondition => "source_changed",
        Code::PermissionDenied | Code::Unauthenticated => "permission_denied",
        Code::Unimplemented => "unsupported",
        Code::InvalidArgument => "invalid_argument",
        Code::Unavailable | Code::DeadlineExceeded | Code::Cancelled => "source_unavailable",
        _ => "source_protocol_error",
    };
    // Do not forward arbitrary server text (may contain credentials or addresses).
    error(kind, format!("Event source returned {:?}", s.code()))
}
#[derive(Clone)]
struct Endpoint {
    address: String,
    timeout_ms: u64,
    catalog: wire::ListEventTypesResponse,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Add {
    id: String,
    endpoint: String,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}
fn default_timeout() -> u64 {
    5000
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Subscribe {
    endpoint_id: String,
    subscription_id: String,
    consumer_id: String,
    #[serde(default)]
    event_types: Vec<String>,
    #[serde(default)]
    after_cursor: String,
    #[serde(default)]
    stream_id: Option<String>,
    #[serde(default = "default_capacity")]
    capacity: usize,
}
fn default_capacity() -> usize {
    32
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Poll {
    subscription_id: String,
    #[serde(default = "default_capacity")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    subscription_id: String,
    cursor: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubscriptionId {
    subscription_id: String,
}
struct Pending {
    cursor: String,
    event_id: String,
    value: Value,
    delivered: bool,
}
struct State {
    pending: VecDeque<Pending>,
    checkpoint: String,
    recent: VecDeque<String>,
    status: &'static str,
    failure: Option<&'static str>,
}
struct Subscription {
    endpoint_id: String,
    endpoint: Endpoint,
    request: wire::SubscribeRequest,
    capacity: usize,
    state: Mutex<State>,
    changed: Notify,
    cancel: watch::Sender<bool>,
    ack_gate: tokio::sync::Mutex<()>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Subscription {
    fn stop(&self) {
        self.cancel.send_replace(true);
        self.changed.notify_one();
    }
    fn status(&self, status: &'static str, failure: Option<&'static str>) {
        let mut s = self.state.lock().unwrap();
        s.status = status;
        s.failure = failure;
    }
}
#[derive(Default)]
struct Catalog {
    endpoints: BTreeMap<String, Endpoint>,
    subscriptions: BTreeMap<String, Arc<Subscription>>,
}
#[derive(Default)]
pub struct EventSourceHub {
    catalog: Mutex<Catalog>,
    closed: AtomicBool,
}
impl Drop for EventSourceHub {
    fn drop(&mut self) {
        self.close();
    }
}
impl EventSourceHub {
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        for sub in self.catalog.lock().unwrap().subscriptions.values() {
            sub.stop();
        }
    }
    pub async fn invoke(&self, command: &str, payload: Value) -> Result<Value> {
        if self.closed.load(Ordering::Acquire) {
            return Err(error("bad_state", "Event ingestion is closed"));
        }
        match command {
            "runtime.add_event_source" => self.add(decode(payload)?).await,
            "runtime.get_event_source_definitions" => {
                let _: BTreeMap<String, Value> = decode(payload.clone())?;
                if payload != json!({}) {
                    return Err(invalid("Expected empty payload"));
                }
                let c = self.catalog.lock().unwrap();
                Ok(
                    json!({"schema":"agent-runtime-event-sources/v1","sources":c.endpoints.iter().map(|(k,e)| catalog_json(k, &e.catalog)).collect::<Vec<_>>()}),
                )
            }
            "runtime.subscribe_events" => self.subscribe(decode(payload)?),
            "runtime.poll_source_events" => {
                let p: Poll = decode(payload)?;
                if !(1..=128).contains(&p.limit) {
                    return Err(invalid("limit must be 1..128"));
                }
                let sub = self.sub(&p.subscription_id)?;
                let mut s = sub.state.lock().unwrap();
                // Poll is retryable: repeat unacknowledged deliveries rather
                // than losing an event when the host misses a poll response.
                let events: Vec<_> = s
                    .pending
                    .iter_mut()
                    .take(p.limit)
                    .map(|p| {
                        p.delivered = true;
                        p.value.clone()
                    })
                    .collect();
                Ok(
                    json!({"schema":"agent-runtime-source-events/v1","subscription_id":p.subscription_id,"status":s.status,"error_kind":s.failure,"acknowledged_cursor":s.checkpoint,"pending_count":s.pending.len(),"capacity":sub.capacity,"events":events}),
                )
            }
            "runtime.acknowledge_events" => self.ack(decode(payload)?).await,
            "runtime.unsubscribe_events" => {
                let p: SubscriptionId = decode(payload)?;
                let sub = self.sub(&p.subscription_id)?;
                sub.stop();
                let task = sub.task.lock().unwrap().take();
                if let Some(mut task) = task {
                    if tokio::time::timeout(Duration::from_secs(2), &mut task)
                        .await
                        .is_err()
                    {
                        task.abort();
                        let _ = task.await;
                    }
                }
                let mut c = self.catalog.lock().unwrap();
                if c.subscriptions
                    .get(&p.subscription_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &sub))
                {
                    c.subscriptions.remove(&p.subscription_id);
                }
                Ok(json!({"unsubscribed":true}))
            }
            _ => Err(error("unsupported", "Unknown event ingestion command")),
        }
    }
    fn sub(&self, name: &str) -> Result<Arc<Subscription>> {
        self.catalog
            .lock()
            .unwrap()
            .subscriptions
            .get(name)
            .cloned()
            .ok_or_else(|| error("not_found", "Subscription not found"))
    }
    async fn add(&self, p: Add) -> Result<Value> {
        if !id(&p.id) || p.endpoint.len() > 4096 || !(1..=60000).contains(&p.timeout_ms) {
            return Err(invalid("Invalid endpoint id, address or timeout_ms"));
        }
        if !p.endpoint.starts_with("http://") && !p.endpoint.starts_with("https://") {
            return Err(invalid("Expected HTTP(S) gRPC address"));
        }
        {
            let c = self.catalog.lock().unwrap();
            if c.endpoints.contains_key(&p.id) {
                return Err(error(
                    "event_source_exists",
                    "Endpoint id already registered",
                ));
            }
            if c.endpoints.len() >= 32 {
                return Err(error("resource_limit", "At most 32 event endpoints"));
            }
        }
        let catalog = tokio::time::timeout(Duration::from_millis(p.timeout_ms), async {
            let mut client = client(&p.endpoint).await?;
            client
                .list_event_types(wire::ListEventTypesRequest {
                    accepted_schema: vec![SCHEMA.into()],
                })
                .await
                .map(|r| r.into_inner())
                .map_err(rpc_error)
        })
        .await
        .map_err(|_| error("source_unavailable", "Event discovery timed out"))??;
        validate_catalog(&catalog)?;
        let result = catalog_json(&p.id, &catalog);
        let mut c = self.catalog.lock().unwrap();
        if self.closed.load(Ordering::Acquire) {
            return Err(error("bad_state", "Event ingestion is closed"));
        }
        if c.endpoints.contains_key(&p.id) {
            return Err(error(
                "event_source_exists",
                "Endpoint id already registered",
            ));
        }
        if c.endpoints.len() >= 32 {
            return Err(error("resource_limit", "At most 32 event endpoints"));
        }
        c.endpoints.insert(
            p.id,
            Endpoint {
                address: p.endpoint,
                timeout_ms: p.timeout_ms,
                catalog,
            },
        );
        Ok(result)
    }
    fn subscribe(&self, p: Subscribe) -> Result<Value> {
        if !id(&p.subscription_id)
            || !id(&p.consumer_id)
            || !(1..=128).contains(&p.capacity)
            || p.after_cursor.len() > 2048
        {
            return Err(invalid(
                "Invalid subscription id, consumer id, capacity or cursor",
            ));
        }
        let mut c = self.catalog.lock().unwrap();
        if self.closed.load(Ordering::Acquire) {
            return Err(error("bad_state", "Event ingestion is closed"));
        }
        let endpoint = c
            .endpoints
            .get(&p.endpoint_id)
            .cloned()
            .ok_or_else(|| error("not_found", "Event endpoint not found"))?;
        if c.subscriptions.contains_key(&p.subscription_id)
            || c.subscriptions.values().any(|s| {
                s.endpoint.catalog.source_id == endpoint.catalog.source_id
                    && s.request.consumer_id == p.consumer_id
            })
        {
            return Err(error(
                "subscription_exists",
                "Subscription id or source/consumer is already active",
            ));
        }
        if c.subscriptions.len() >= 16 {
            return Err(error("resource_limit", "At most 16 subscriptions"));
        }
        if !endpoint.catalog.replayable && !p.after_cursor.is_empty() {
            return Err(invalid("Live-only sources cannot resume a cursor"));
        }
        if (!p.after_cursor.is_empty() || p.stream_id.is_some())
            && p.stream_id.as_deref() != Some(endpoint.catalog.stream_id.as_str())
        {
            return Err(error(
                "source_changed",
                "Resume requires the original stream_id",
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        for name in &p.event_types {
            if !names.insert(name) || !endpoint.catalog.event_types.iter().any(|e| &e.name == name)
            {
                return Err(invalid("Unknown or duplicate event type"));
            }
        }
        let (cancel, rx) = watch::channel(false);
        let sub = Arc::new(Subscription {
            endpoint_id: p.endpoint_id,
            request: wire::SubscribeRequest {
                source_id: endpoint.catalog.source_id.clone(),
                stream_id: endpoint.catalog.stream_id.clone(),
                consumer_id: p.consumer_id,
                event_types: p.event_types,
                after_cursor: p.after_cursor.clone(),
            },
            endpoint,
            capacity: p.capacity,
            state: Mutex::new(State {
                pending: VecDeque::new(),
                checkpoint: p.after_cursor,
                recent: VecDeque::new(),
                status: "connecting",
                failure: None,
            }),
            changed: Notify::new(),
            cancel,
            ack_gate: tokio::sync::Mutex::new(()),
            task: Mutex::new(None),
        });
        let copy = sub.clone();
        *sub.task.lock().unwrap() = Some(tokio::spawn(async move { run(copy, rx).await }));
        c.subscriptions.insert(p.subscription_id.clone(), sub);
        Ok(json!({"subscription_id":p.subscription_id,"status":"connecting"}))
    }
    async fn ack(&self, p: Ack) -> Result<Value> {
        let sub = self.sub(&p.subscription_id)?;
        let _gate = sub.ack_gate.lock().await;
        {
            let s = sub.state.lock().unwrap();
            if p.cursor.is_empty() {
                return Err(invalid("cursor is required"));
            }
            if p.cursor == s.checkpoint {
                return Ok(json!({"cursor":p.cursor}));
            }
            let index = s
                .pending
                .iter()
                .position(|v| v.cursor == p.cursor)
                .ok_or_else(|| invalid("Cursor was not delivered by this subscription"))?;
            if s.pending.iter().take(index + 1).any(|v| !v.delivered) {
                return Err(invalid("Cannot acknowledge unread events"));
            }
        }
        let result = tokio::time::timeout(Duration::from_millis(sub.endpoint.timeout_ms), async {
            let mut client = client(&sub.endpoint.address).await?;
            client
                .acknowledge(wire::AcknowledgeRequest {
                    source_id: sub.request.source_id.clone(),
                    stream_id: sub.request.stream_id.clone(),
                    consumer_id: sub.request.consumer_id.clone(),
                    cursor: p.cursor.clone(),
                })
                .await
                .map(|r| r.into_inner())
                .map_err(rpc_error)
        })
        .await
        .map_err(|_| error("source_unavailable", "Acknowledgement timed out"))??;
        if result.cursor != p.cursor {
            return Err(error(
                "source_protocol_error",
                "Acknowledgement cursor mismatch",
            ));
        }
        let mut s = sub.state.lock().unwrap();
        while let Some(v) = s.pending.pop_front() {
            s.recent.push_back(v.cursor.clone());
            if s.recent.len() > 256 {
                s.recent.pop_front();
            }
            if v.cursor == p.cursor {
                break;
            }
        }
        s.checkpoint = p.cursor.clone();
        sub.changed.notify_one();
        Ok(json!({"cursor":p.cursor}))
    }
}
async fn client(address: &str) -> Result<Client<Channel>> {
    Client::connect(address.to_owned())
        .await
        .map(|c| c.max_decoding_message_size(256 * 1024))
        .map_err(|_| error("source_unavailable", "Cannot connect to event source"))
}
fn validate_catalog(c: &wire::ListEventTypesResponse) -> Result<()> {
    if c.schema != SCHEMA
        || !id(&c.source_id)
        || !id(&c.stream_id)
        || c.event_types.is_empty()
        || c.event_types.len() > 128
    {
        return Err(error("source_protocol_error", "Invalid event catalog"));
    }
    if !matches!(
        (c.replayable, c.durability.as_str()),
        (false, "live") | (true, "memory") | (true, "durable")
    ) {
        return Err(error(
            "source_protocol_error",
            "Invalid replay/durability declaration",
        ));
    }
    let mut names = std::collections::BTreeSet::new();
    for e in &c.event_types {
        if !id(&e.name)
            || !id(&e.schema_version)
            || !names.insert(&e.name)
            || e.description.len() > 4096
            || e.data_schema_json.len() > MAX_BYTES
            || !serde_json::from_str::<Value>(&e.data_schema_json).is_ok_and(|v| v.is_object())
        {
            return Err(error(
                "source_protocol_error",
                "Invalid event type descriptor",
            ));
        }
    }
    Ok(())
}
fn catalog_json(id: &str, c: &wire::ListEventTypesResponse) -> Value {
    json!({"endpoint_id":id,"source_id":c.source_id,"stream_id":c.stream_id,"replayable":c.replayable,"durability":c.durability,"retention_ms":c.retention_ms,"persistence":"host_managed","event_types":c.event_types.iter().map(|e|json!({"name":e.name,"description":e.description,"schema_version":e.schema_version,"data_schema":serde_json::from_str::<Value>(&e.data_schema_json).unwrap_or(Value::Null)})).collect::<Vec<_>>()})
}
fn ingest(sub: &Subscription, e: wire::SourceEvent) -> Result<()> {
    let descriptor = sub
        .endpoint
        .catalog
        .event_types
        .iter()
        .find(|t| t.name == e.event_type)
        .ok_or_else(|| error("source_protocol_error", "Undeclared event type"))?;
    if e.source_id != sub.request.source_id || e.stream_id != sub.request.stream_id {
        return Err(error("source_changed", "Source or stream identity changed"));
    }
    if !id(&e.event_id)
        || e.cursor.is_empty()
        || e.cursor.len() > 2048
        || e.data_json.len() > MAX_BYTES
        || e.schema_version != descriptor.schema_version
        || chrono::DateTime::parse_from_rfc3339(&e.occurred_at).is_err()
        || (!sub.request.event_types.is_empty() && !sub.request.event_types.contains(&e.event_type))
    {
        return Err(error(
            "source_protocol_error",
            "Invalid source event envelope",
        ));
    }
    let data: Value = serde_json::from_str(&e.data_json)
        .map_err(|_| error("source_protocol_error", "Invalid event JSON"))?;
    let mut s = sub.state.lock().unwrap();
    if e.cursor == s.checkpoint || s.recent.contains(&e.cursor) {
        return Ok(());
    }
    if let Some(old) = s
        .pending
        .iter()
        .find(|v| v.cursor == e.cursor || v.event_id == e.event_id)
    {
        if old.cursor == e.cursor && old.event_id == e.event_id && old.value["data"] == data {
            return Ok(());
        }
        return Err(error("source_protocol_error", "Event identity was reused"));
    }
    s.pending.push_back(Pending {cursor:e.cursor.clone(),event_id:e.event_id.clone(),delivered:false,value:json!({"schema":"corework-source-event/v1","endpoint_id":sub.endpoint_id,"source_id":e.source_id,"stream_id":e.stream_id,"event_id":e.event_id,"event_type":e.event_type,"schema_version":e.schema_version,"occurred_at":e.occurred_at,"cursor":e.cursor,"data":data})});
    Ok(())
}
async fn session(sub: &Subscription) -> Result<()> {
    let mut request = sub.request.clone();
    request.after_cursor = sub.state.lock().unwrap().checkpoint.clone();
    let mut stream = tokio::time::timeout(Duration::from_millis(sub.endpoint.timeout_ms), async {
        let mut client = client(&sub.endpoint.address).await?;
        client
            .subscribe(request)
            .await
            .map(|r| r.into_inner())
            .map_err(rpc_error)
    })
    .await
    .map_err(|_| error("source_unavailable", "Subscription handshake timed out"))??;
    sub.status("connected", None);
    loop {
        // Pending includes polled but unacknowledged events. Polling alone cannot
        // relieve backpressure or advance the reconnect checkpoint.
        while sub.state.lock().unwrap().pending.len() >= sub.capacity {
            sub.changed.notified().await;
        }
        let frame = tokio::time::timeout(Duration::from_secs(30), stream.message())
            .await
            .map_err(|_| error("source_unavailable", "Event heartbeat timed out"))?
            .map_err(rpc_error)?
            .ok_or_else(|| error("source_unavailable", "Event stream disconnected"))?;
        match frame.message {
            Some(Message::Event(e)) => ingest(sub, e)?,
            Some(Message::Heartbeat(_)) => {}
            None => return Err(error("source_protocol_error", "Empty event frame")),
        }
    }
}
async fn run(sub: Arc<Subscription>, mut cancel: watch::Receiver<bool>) {
    let mut retry_ms = 100;
    loop {
        let result =
            tokio::select! {biased; _=cancel.changed()=>break, result=session(&sub)=>result};
        let e = result.unwrap_err();
        if e.kind != "source_unavailable" || !sub.endpoint.catalog.replayable {
            sub.status(
                "failed",
                Some(
                    if e.kind == "source_unavailable" && !sub.endpoint.catalog.replayable {
                        "live_source_disconnected"
                    } else {
                        e.kind
                    },
                ),
            );
            return;
        }
        sub.status("reconnecting", Some(e.kind));
        tokio::select! {biased; _=cancel.changed()=>break, _=tokio::time::sleep(Duration::from_millis(retry_ms))=>{}}
        retry_ms = (retry_ms * 2).min(5000);
    }
    sub.status("stopped", None);
}
