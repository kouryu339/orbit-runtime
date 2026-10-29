use super::*;
use corework_agent_tool::proto::{
    self,
    agent_tool_service_server::{AgentToolService, AgentToolServiceServer},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::{Request, Response, Status};

#[derive(Clone)]
struct ProbeService {
    tools: Vec<proto::ToolDescriptor>,
    value: String,
    gate: Arc<Semaphore>,
    started: std_mpsc::Sender<()>,
    calls: Arc<AtomicUsize>,
}

#[tonic::async_trait]
impl AgentToolService for ProbeService {
    async fn list_tools(
        &self,
        _: Request<proto::ListToolsRequest>,
    ) -> Result<Response<proto::ListToolsResponse>, Status> {
        let _ = self.started.send(());
        self.gate.acquire().await.unwrap().forget();
        Ok(Response::new(proto::ListToolsResponse {
            schema: corework_agent_tool::SCHEMA.into(),
            tools: self.tools.clone(),
        }))
    }

    type ExecuteStream = ReceiverStream<Result<proto::ToolStreamMessage, Status>>;
    async fn execute(
        &self,
        request: Request<tonic::Streaming<proto::ToolStreamMessage>>,
    ) -> Result<Response<Self::ExecuteStream>, Status> {
        let mut stream = request.into_inner();
        let first = stream
            .message()
            .await?
            .ok_or_else(|| Status::invalid_argument("execute request missing"))?;
        assert!(matches!(
            first.message,
            Some(proto::tool_stream_message::Message::ExecuteRequest(_))
        ));
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.send(Ok(proto::ToolStreamMessage {
            call_id: first.call_id,
            message: Some(proto::tool_stream_message::Message::AiOutput(
                proto::AiOutput {
                    result_json: json!({"value": self.value}).to_string(),
                    to_ai: self.value.clone(),
                    error_code: 1,
                },
            )),
        }))
        .await
        .unwrap();
        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

struct Server {
    _rt: Runtime,
    address: String,
    service: ProbeService,
    started: std_mpsc::Receiver<()>,
}

fn descriptor(name: &str) -> proto::ToolDescriptor {
    proto::ToolDescriptor {
        name: name.into(),
        description: "Return a fixture value.".into(),
        readonly: true,
        idempotent: true,
        outputs: vec![proto::ToolOutputField {
            name: "value".into(),
            field_type: "String".into(),
            description: "Fixture value.".into(),
        }],
        ..Default::default()
    }
}

impl Server {
    fn new(names: &[&str], value: &str, permits: usize) -> Self {
        Self::with_tools(
            names.iter().map(|name| descriptor(name)).collect(),
            value,
            permits,
        )
    }
    fn with_tools(tools: Vec<proto::ToolDescriptor>, value: &str, permits: usize) -> Self {
        let rt = Runtime::new().unwrap();
        let listener = rt
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let (tx, started) = std_mpsc::channel();
        let service = ProbeService {
            tools,
            value: value.into(),
            gate: Arc::new(Semaphore::new(permits)),
            started: tx,
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let hosted = service.clone();
        rt.spawn(async move {
            tonic::transport::Server::builder()
                .add_service(AgentToolServiceServer::new(hosted))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        Self {
            _rt: rt,
            address,
            service,
            started,
        }
    }
    fn request(&self, id: &str) -> Value {
        json!({"schema":"agent-runtime-rpc-endpoint-add/v1","endpoint":{"id":id,"protocol":"grpc","endpoint":self.address,"timeout_ms":2000}})
    }
}

fn add(facade: &mut RuntimeFacade, request: Value) -> Result<Value, RuntimeError> {
    let discovered = facade.prepare_rpc_endpoint_addition(request)?.discover()?;
    facade.commit_rpc_endpoint_addition(discovered, "rpc-fixture-add")
}

fn rpc_runtime(root: &Path) -> RuntimeFacade {
    let mut facade = dynamic_cluster_runtime(root);
    facade.config.workflow.auto_load_dir = Some(root.join("workflows"));
    facade
}

#[test]
fn dynamic_rpc_addition_rejects_conflicts_and_invalid_batches_without_leaks() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("dynamic-rpc-atomic");
    let mut facade = rpc_runtime(&root);
    let initial = Server::new(&["DynamicProbe"], "original", 1);
    assert!(matches!(
        facade.prepare_rpc_endpoint_addition(initial.request("original")),
        Err(RuntimeError::NotStarted)
    ));
    facade.start().unwrap();
    let result = add(&mut facade, initial.request("original")).unwrap();
    assert_eq!(result["tool_names"], json!(["DynamicProbe"]));
    assert_eq!(result["catalog_revision"], 1);
    let before = facade.tool_definitions().unwrap();
    // Identical re-registration is a conflict, not an update or a second discovery.
    assert!(matches!(
        facade.prepare_rpc_endpoint_addition(initial.request("original")),
        Err(RuntimeError::RpcAdditionConflict {
            kind: "rpc_endpoint_exists",
            ..
        })
    ));
    for names in [
        vec!["FreshProbe", "DynamicProbe"],
        vec!["FreshProbe", "ToolDescribe"],
        vec!["FreshProbe", "WorkflowRef_reserved"],
        vec!["FreshProbe", "FreshProbe"],
    ] {
        let conflicting = Server::new(&names, "replacement", 1);
        assert!(add(&mut facade, conflicting.request("rejected")).is_err());
        assert_eq!(facade.tool_definitions().unwrap(), before);
        assert!(facade
            .registries
            .resources
            .as_ref()
            .unwrap()
            .rpc_pool
            .get("rejected")
            .is_none());
        assert!(facade.tool_registry.get_dynamic("FreshProbe").is_none());
    }
    let mut malformed = descriptor("InvalidProbe");
    malformed.required_capabilities = vec!["unsupported.fixture.capability".into()];
    let invalid = Server::with_tools(vec![descriptor("FreshProbe"), malformed], "invalid", 1);
    assert!(add(&mut facade, invalid.request("invalid")).is_err());
    let stalled = Server::new(&["TimedOutProbe"], "timeout", 0);
    let mut timeout = stalled.request("timeout");
    timeout["endpoint"]["timeout_ms"] = json!(30);
    assert!(add(&mut facade, timeout).is_err());
    assert_eq!(facade.tool_definitions().unwrap(), before);
    assert_eq!(initial.service.calls.load(Ordering::SeqCst), 0);
    let endpoints: Value =
        serde_json::from_str(&facade.rpc_endpoint_definitions().unwrap()).unwrap();
    assert_eq!(
        endpoints["endpoints"][0]["verification_scope"],
        "add_list_tools"
    );
    assert_eq!(endpoints["endpoints"].as_array().unwrap().len(), 1);
    let changes = facade
        .event_log
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == RPC_CATALOG_CHANGED_EVENT)
        .count();
    assert_eq!(changes, 1);
    facade.shutdown().unwrap();
}

#[test]
fn dynamic_rpc_addition_reaches_existing_contexts_and_workflow_without_granting_access() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("dynamic-rpc-workflow");
    let mut facade = rpc_runtime(&root);
    facade.start().unwrap();
    facade
        .add_agent_cluster(&dynamic_cluster_registration("rpc-team", "cluster_role_a"))
        .unwrap();
    let conversation = facade
        .spawn_conversation(&json!({"cluster_id":"rpc-team"}).to_string())
        .unwrap();
    let manager = facade.manager().unwrap();
    let old_context = manager
        .unit()
        .create_context()
        .with_conversation_id(&conversation.conversation_id);
    old_context.set("agent_id", "primary").unwrap();
    let old_catalog = ai_assistant::systems::workflow::WorkflowRuntimeToolCatalog::from_registry(
        facade.tool_registry.clone(),
    );
    assert!(old_context.get_dynamic_system("LateProbe").is_err());
    let source = r#"{"schema":"orbit.workflow.resource/v1","id":"late-rpc-flow","metadata":{"name":"Late RPC","description":"Exercise a newly added RPC node","extensions":{}},"state":"draft","revision":1,"logic":{"schema":"orbit.workflow.logic/v1","steps":[{"id":1,"call":"LateProbe","args":{}}],"returns":{"value":{"value":1.value}}}}"#;
    assert!(facade.register_logic_resource(source, true).is_err());
    let server = Server::new(&["LateProbe"], "late-value", 1);
    add(&mut facade, server.request("late")).unwrap();
    let denied = facade
        .rt
        .block_on(ai_assistant::tool_runner::execute_single(
            "LateProbe",
            &old_context,
        ));
    assert!(!denied.success);
    assert_eq!(server.service.calls.load(Ordering::SeqCst), 0);
    let cache = facade
        .rt
        .block_on(manager.default_agent_cache(&conversation.conversation_id))
        .unwrap();
    assert!(!facade
        .rt
        .block_on(AssistantContext::get_active_tools(&cache))
        .unwrap()
        .contains(&"LateProbe".into()));
    let definitions = ai_assistant::tool_schema::definitions_for_active_tools_in_registry(
        &["LateProbe".into()],
        false,
        false,
        Some(old_context.registry()),
    )
    .unwrap();
    assert_eq!(definitions.len(), 1);
    assert!(
        ai_assistant::tool_schema::describe_registered_tools_in_registry(
            &["LateProbe".into()],
            Some(old_context.registry())
        )
        .is_ok()
    );
    assert_eq!(old_catalog.tools().len(), 1);
    let output = facade
        .rt
        .block_on(
            old_context
                .get_dynamic_system("LateProbe")
                .unwrap()
                .execute_dynamic(HashMap::new(), &old_context),
        )
        .unwrap();
    assert_eq!(output["result"]["value"], "late-value");
    facade.register_logic_resource(source, true).unwrap();
    let output = crate::invoke_command(
        &mut facade,
        "workflow.execute",
        json!({"id":"late-rpc-flow"}).as_object().unwrap(),
        "late-rpc-run",
    )
    .unwrap();
    assert_eq!(
        output["result"]["outputs"]["value"], "late-value",
        "{output}"
    );
    assert_eq!(server.service.calls.load(Ordering::SeqCst), 2);
    facade
        .close_conversation(&conversation.conversation_id)
        .unwrap();
    facade.shutdown().unwrap();
}

#[test]
fn dynamic_rpc_scoped_catalogs_resolve_same_names_independently() {
    let _guard = runtime_start_test_guard();
    let mut left = rpc_runtime(&unique_test_dir("rpc-isolated-a"));
    left.start().unwrap();
    let left_context = left.manager().unwrap().unit().create_context();
    let a = Server::new(&["IsolatedProbe"], "A", 1);
    add(&mut left, a.request("same-id")).unwrap();
    // Full Runtime startup still has a pre-existing global Workflow ownership
    // constraint. Exercise independent RPC scopes without claiming otherwise.
    let right_registry = Arc::new(corework::system::SystemRegistry::isolated_runtime());
    let b = Server::new(&["IsolatedProbe"], "B", 1);
    let endpoint = resource_rpc_endpoint_to_runtime(
        serde_json::from_value(b.request("same-id")["endpoint"].clone()).unwrap(),
        None,
    )
    .unwrap();
    let prepared = left.rt.block_on(prepare_rpc_tools(vec![endpoint])).unwrap();
    right_registry
        .append_runtime_tools(prepared.entries)
        .unwrap();
    let right_context = left
        .manager()
        .unwrap()
        .unit()
        .create_context()
        .set_registry(right_registry);
    for (context, expected) in [
        (&left_context, "A"),
        (&right_context, "B"),
        (&left_context, "A"),
    ] {
        let output = left
            .rt
            .block_on(
                context
                    .get_dynamic_system("IsolatedProbe")
                    .unwrap()
                    .execute_dynamic(HashMap::new(), context),
            )
            .unwrap();
        assert_eq!(output["result"]["value"], expected);
        assert_eq!(context.registry().runtime_tools()[0].endpoint_id, "same-id");
    }
    assert!(ai_assistant::runtime_tools::get_runtime_tool("IsolatedProbe").is_none());
    left.shutdown().unwrap();
}

fn install_test_handle(
    mut facade: RuntimeFacade,
) -> (crate::AgentRuntimeHandle, Arc<crate::HandleEntry>) {
    let (sender, receiver) = std_mpsc::channel();
    facade.set_event_sender(sender);
    let entry = Arc::new(crate::HandleEntry {
        facade: std::sync::RwLock::new(facade),
        events: StdMutex::new(receiver),
        lifecycle: StdMutex::new(crate::Lifecycle {
            state: crate::LifecycleState::Open,
            active_calls: 0,
            shutdown_started: false,
            shutdown_error: None,
        }),
        lifecycle_changed: std::sync::Condvar::new(),
    });
    let handle = crate::NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    crate::handles()
        .lock()
        .unwrap()
        .insert(handle, entry.clone());
    (handle, entry)
}

fn invoke_ffi(handle: crate::AgentRuntimeHandle, ty: &str, payload: Value) -> Value {
    let text = std::ffi::CString::new(
        json!({"schema":"agent-runtime-command/v1","type":ty,"payload":payload}).to_string(),
    )
    .unwrap();
    let mut out = std::ptr::null_mut();
    crate::agent_runtime_invoke_v1(handle, text.as_ptr(), &mut out);
    assert!(!out.is_null());
    let result =
        serde_json::from_str(unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap()).unwrap();
    crate::agent_runtime_free_string_v1(out);
    result
}

#[test]
fn dynamic_rpc_ffi_discovery_releases_lock_and_concurrent_duplicate_has_one_winner() {
    let _guard = runtime_start_test_guard();
    let mut facade = rpc_runtime(&unique_test_dir("rpc-ffi-concurrent"));
    facade.start().unwrap();
    let (handle, entry) = install_test_handle(facade);
    let server = Server::new(&["ConcurrentProbe"], "race", 0);
    let mut workers = Vec::new();
    for _ in 0..2 {
        let request = server.request("concurrent");
        workers.push(std::thread::spawn(move || {
            invoke_ffi(handle, "runtime.add_rpc_endpoint", request)
        }));
    }
    for _ in 0..2 {
        server.started.recv_timeout(Duration::from_secs(2)).unwrap();
    }
    let began = std::time::Instant::now();
    let query = invoke_ffi(handle, "runtime.get_tool_definitions", json!({}));
    assert_eq!(query["ok"], true);
    assert!(began.elapsed() < Duration::from_millis(500));
    server.service.gate.add_permits(2);
    let outcomes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes.iter().filter(|value| value["ok"] == true).count(),
        1,
        "{outcomes:?}"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| value["error"]["kind"] == "rpc_endpoint_exists")
            .count(),
        1
    );
    assert_eq!(entry.facade.read().unwrap().runtime_tools.len(), 1);
    assert_eq!(
        crate::agent_runtime_shutdown_v1(handle, 10_000),
        crate::AGENT_RUNTIME_OK
    );
    assert_eq!(
        crate::agent_runtime_destroy_v1(handle),
        crate::AGENT_RUNTIME_OK
    );
}

#[test]
fn dynamic_rpc_ffi_closing_runtime_rejects_inflight_discovery_commit() {
    let _guard = runtime_start_test_guard();
    let mut facade = rpc_runtime(&unique_test_dir("rpc-ffi-closing"));
    facade.start().unwrap();
    let (handle, entry) = install_test_handle(facade);
    let server = Server::new(&["ClosingProbe"], "closing", 0);
    let request = server.request("closing");
    let worker =
        std::thread::spawn(move || invoke_ffi(handle, "runtime.add_rpc_endpoint", request));
    server.started.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        crate::agent_runtime_shutdown_v1(handle, 0),
        crate::AGENT_RUNTIME_ERR_TIMEOUT
    );
    server.service.gate.add_permits(1);
    let result = worker.join().unwrap();
    assert_eq!(result["error"]["kind"], "bad_state", "{result}");
    assert_eq!(
        crate::agent_runtime_shutdown_v1(handle, 10_000),
        crate::AGENT_RUNTIME_OK
    );
    assert!(entry
        .facade
        .read()
        .unwrap()
        .event_log
        .lock()
        .unwrap()
        .iter()
        .all(|event| event["type"] != RPC_CATALOG_CHANGED_EVENT));
    assert_eq!(
        crate::agent_runtime_destroy_v1(handle),
        crate::AGENT_RUNTIME_OK
    );
}
