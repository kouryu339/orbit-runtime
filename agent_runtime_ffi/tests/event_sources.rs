//! Real TCP/gRPC regressions for the independent bounded ingestion path.
use corework::rpc_event::EventSourceHub;
use corework_agent_tool::events_proto::{
    self as p,
    event_source_service_server::{EventSourceService, EventSourceServiceServer},
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::{Request, Response, Status};

#[derive(Clone)]
struct Source {
    mode: &'static str,
    attempts: Arc<AtomicUsize>,
    acks: Arc<Mutex<Vec<String>>>,
}
#[tonic::async_trait]
impl EventSourceService for Source {
    async fn list_event_types(
        &self,
        _: Request<p::ListEventTypesRequest>,
    ) -> Result<Response<p::ListEventTypesResponse>, Status> {
        if self.mode == "slow" {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        Ok(Response::new(p::ListEventTypesResponse {
            schema: "corework-event-source/v1".into(),
            source_id: "wechat:account-a".into(),
            stream_id: "journal-1".into(),
            replayable: self.mode != "live",
            durability: if self.mode == "live" {
                "live"
            } else {
                "memory"
            }
            .into(),
            retention_ms: 60000,
            event_types: vec![p::EventType {
                name: "message.received".into(),
                description: "Incoming message".into(),
                schema_version: "1".into(),
                data_schema_json: "{\"type\":\"object\"}".into(),
            }],
        }))
    }
    type SubscribeStream = ReceiverStream<Result<p::EventFrame, Status>>;
    async fn subscribe(
        &self,
        r: Request<p::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if self.mode == "expired" {
            return Err(Status::out_of_range("expired"));
        }
        if self.mode == "changed" {
            return Err(Status::failed_precondition("changed"));
        }
        let from = r
            .into_inner()
            .after_cursor
            .parse::<usize>()
            .unwrap_or_default();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let mode = self.mode;
        tokio::spawn(async move {
            for i in from + 1..=5 {
                let e = p::SourceEvent {
                    source_id: "wechat:account-a".into(),
                    stream_id: "journal-1".into(),
                    event_id: format!("event-{i}"),
                    event_type: "message.received".into(),
                    schema_version: "1".into(),
                    occurred_at: "2026-10-30T01:00:00Z".into(),
                    cursor: i.to_string(),
                    data_json: if mode == "invalid" {
                        "invalid".into()
                    } else {
                        format!("{{\"value\":{i}}}")
                    },
                };
                if tx
                    .send(Ok(p::EventFrame {
                        message: Some(p::event_frame::Message::Event(e)),
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
                if mode == "live" || (mode == "reconnect" && n == 0) {
                    let _ = tx.send(Err(Status::unavailable("disconnected"))).await;
                    return;
                }
            }
            tx.closed().await;
        });
        Ok(Response::new(ReceiverStream::new(rx)))
    }
    async fn acknowledge(
        &self,
        r: Request<p::AcknowledgeRequest>,
    ) -> Result<Response<p::AcknowledgeResponse>, Status> {
        let cursor = r.into_inner().cursor;
        if self.mode == "ackfail" {
            return Err(Status::unavailable("lost ack"));
        }
        self.acks.lock().unwrap().push(cursor.clone());
        Ok(Response::new(p::AcknowledgeResponse { cursor }))
    }
}
struct Fixture {
    hub: EventSourceHub,
    source: Source,
    server: tokio::task::JoinHandle<()>,
    address: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.hub.close();
        self.server.abort();
    }
}
impl Fixture {
    async fn new(mode: &'static str) -> Self {
        let source = Source {
            mode,
            attempts: Default::default(),
            acks: Default::default(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let svc = source.clone();
        let server = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(EventSourceServiceServer::new(svc))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        Self {
            hub: EventSourceHub::default(),
            source,
            server,
            address,
        }
    }
    async fn add(&self) {
        self.hub
            .invoke(
                "runtime.add_event_source",
                json!({"id":"wechat","endpoint":self.address}),
            )
            .await
            .unwrap();
    }
    async fn subscribe(&self, capacity: usize) {
        self.hub.invoke("runtime.subscribe_events",json!({"endpoint_id":"wechat","subscription_id":"sub","consumer_id":"project-a","capacity":capacity})).await.unwrap();
    }
    async fn poll(&self) -> Value {
        self.hub
            .invoke(
                "runtime.poll_source_events",
                json!({"subscription_id":"sub","limit":128}),
            )
            .await
            .unwrap()
    }
    async fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..200 {
            let v = self.poll().await;
            if predicate(&v) {
                return v;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("Timed out waiting for source state");
    }
}
#[tokio::test]
async fn event_only_discovery_is_append_only_and_catalog_hides_address() {
    let f = Fixture::new("burst").await;
    f.add().await;
    let v = f
        .hub
        .invoke("runtime.get_event_source_definitions", json!({}))
        .await
        .unwrap();
    assert_eq!(v["sources"][0]["source_id"], "wechat:account-a");
    assert!(!v.to_string().contains(&f.address));
    assert_eq!(
        f.hub
            .invoke(
                "runtime.add_event_source",
                json!({"id":"wechat","endpoint":f.address})
            )
            .await
            .unwrap_err()
            .kind,
        "event_source_exists"
    );
}
#[tokio::test]
async fn backpressure_requires_ack_not_poll_and_unread_ack_is_rejected() {
    let f = Fixture::new("burst").await;
    f.add().await;
    f.subscribe(2).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        f.hub
            .invoke(
                "runtime.acknowledge_events",
                json!({"subscription_id":"sub","cursor":"2"})
            )
            .await
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    let v = f.poll().await;
    assert_eq!(v["events"].as_array().unwrap().len(), 2);
    assert_eq!(v["pending_count"], 2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let v = f.poll().await;
    assert_eq!(v["events"].as_array().unwrap().len(), 2);
    assert_eq!(v["events"][0]["cursor"], "1");
    assert_eq!(v["pending_count"], 2);
    assert_eq!(v["acknowledged_cursor"], "");
    for _ in 0..2 {
        f.hub
            .invoke(
                "runtime.acknowledge_events",
                json!({"subscription_id":"sub","cursor":"2"}),
            )
            .await
            .unwrap();
    }
    assert_eq!(f.source.acks.lock().unwrap().len(), 1);
    let v = f
        .wait(|v| !v["events"].as_array().unwrap().is_empty())
        .await;
    assert_eq!(v["events"][0]["cursor"], "3");
    f.hub
        .invoke(
            "runtime.unsubscribe_events",
            json!({"subscription_id":"sub"}),
        )
        .await
        .unwrap();
    assert_eq!(
        f.hub
            .invoke(
                "runtime.poll_source_events",
                json!({"subscription_id":"sub"})
            )
            .await
            .unwrap_err()
            .kind,
        "not_found"
    );
}
#[tokio::test]
async fn reconnect_preserves_unacknowledged_delivery_without_duplicate_queue_entries() {
    let f = Fixture::new("reconnect").await;
    f.add().await;
    f.subscribe(16).await;
    let mut events = vec![];
    for _ in 0..100 {
        let v = f.poll().await;
        events = v["events"].as_array().unwrap().clone();
        if events.len() == 5 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(events.len(), 5);
    assert!(f.source.attempts.load(Ordering::SeqCst) >= 2);
    let ids: std::collections::BTreeSet<_> = events
        .iter()
        .map(|e| e["event_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 5);
    assert_eq!(f.poll().await["acknowledged_cursor"], "");
    f.hub
        .invoke(
            "runtime.acknowledge_events",
            json!({"subscription_id":"sub","cursor":"5"}),
        )
        .await
        .unwrap();
    assert_eq!(f.poll().await["pending_count"], 0);
}
#[tokio::test]
async fn terminal_failures_and_live_only_disconnect_are_visible() {
    for (mode, kind) in [
        ("expired", "cursor_expired"),
        ("changed", "source_changed"),
        ("invalid", "source_protocol_error"),
        ("live", "live_source_disconnected"),
    ] {
        let f = Fixture::new(mode).await;
        f.add().await;
        f.subscribe(16).await;
        let v = f.wait(|v| v["status"] == "failed").await;
        assert_eq!(v["error_kind"], kind);
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(f.source.attempts.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn failed_ack_preserves_pending_and_checkpoint() {
    let f = Fixture::new("ackfail").await;
    f.add().await;
    f.subscribe(16).await;
    f.wait(|v| v["pending_count"] == 5).await;
    assert_eq!(
        f.hub
            .invoke(
                "runtime.acknowledge_events",
                json!({"subscription_id":"sub","cursor":"5"})
            )
            .await
            .unwrap_err()
            .kind,
        "source_unavailable"
    );
    let v = f.poll().await;
    assert_eq!(v["pending_count"], 5);
    assert_eq!(v["acknowledged_cursor"], "");
}
#[tokio::test]
async fn resume_requires_matching_journal_and_uses_exclusive_checkpoint() {
    let f = Fixture::new("burst").await;
    f.add().await;
    assert_eq!(f.hub.invoke("runtime.subscribe_events",json!({"endpoint_id":"wechat","subscription_id":"sub","consumer_id":"project-a","after_cursor":"2"})).await.unwrap_err().kind,"source_changed");
    f.hub.invoke("runtime.subscribe_events",json!({"endpoint_id":"wechat","subscription_id":"sub","consumer_id":"project-a","after_cursor":"2","stream_id":"journal-1"})).await.unwrap();
    let v = f.wait(|v| v["pending_count"] == 3).await;
    assert_eq!(v["events"][0]["cursor"], "3");
    assert_eq!(v["acknowledged_cursor"], "2");
}
#[tokio::test]
async fn discovery_does_not_hold_catalog_lock_and_close_rejects_late_result() {
    let f = Fixture::new("slow").await;
    let add = f.hub.invoke(
        "runtime.add_event_source",
        json!({"id":"wechat","endpoint":f.address}),
    );
    let close = async {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        f.hub
            .invoke("runtime.get_event_source_definitions", json!({}))
            .await
            .unwrap();
        f.hub.close();
    };
    let (r, _) = tokio::join!(add, close);
    assert_eq!(r.unwrap_err().kind, "bad_state");
}
#[tokio::test]
async fn concurrent_registration_has_one_winner_and_consumers_are_exclusive() {
    let f = Fixture::new("burst").await;
    let (a, b) = tokio::join!(
        f.hub.invoke(
            "runtime.add_event_source",
            json!({"id":"wechat","endpoint":f.address})
        ),
        f.hub.invoke(
            "runtime.add_event_source",
            json!({"id":"wechat","endpoint":f.address})
        )
    );
    assert_ne!(a.is_ok(), b.is_ok());
    f.subscribe(1).await;
    assert_eq!(
        f.hub
            .invoke(
                "runtime.subscribe_events",
                json!({"endpoint_id":"wechat","subscription_id":"other","consumer_id":"project-a"})
            )
            .await
            .unwrap_err()
            .kind,
        "subscription_exists"
    );
}
