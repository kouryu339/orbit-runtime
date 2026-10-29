use super::{
    bridge::{HostBridge, HostToolClient},
    events::EventProjector,
};
use ai_assistant::{ConversationManager, ConversationOptions, SkillManager};
use corework::{
    event::{BaseEvent, EventBus, InMemoryEventBus},
    rpc_tool::{
        validate_runtime_tool_set_for_endpoint, RpcEndpointInfo, RpcEndpointRegistry,
        RpcStubSystem, RuntimeToolMetadata, RuntimeToolRegistry,
    },
    workflow::{
        workflows::{
            catalog::WorkflowCatalogCheckpoint, source_store::MemoryWorkflowSourceStore,
            WorkflowExecutionContext, WorkflowResourceKind,
        },
        WorkflowsModule,
    },
    world::FrameworkState,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

type Result<T> = std::result::Result<T, String>;
type ResourceCheckpoint = (WorkflowCatalogCheckpoint, BTreeMap<PathBuf, Vec<u8>>);
const RESOURCE_COMMANDS: &[&str] = &["workflow.register", "workflow.remove", "workflow.promote"];
const COMMANDS: &[&str] = &[
    "runtime.register_resources",
    "runtime.register_llm",
    "runtime.register_agent_cluster",
    "runtime.add_rpc_endpoint",
    "runtime.get_tool_definitions",
    "runtime.get_workflow_node_definitions",
    "runtime.get_rpc_endpoint_definitions",
    "runtime.set_current_model",
    "runtime.set_language",
    "workflow.list",
    "workflow.validate",
    "workflow.register",
    "workflow.remove",
    "workflow.promote",
    "workflow.execute",
    "conversation.spawn",
    "conversation.send_message",
    "conversation.pause",
    "conversation.close",
    "conversation.materialize",
    "conversation.resolve_tool_permission",
    "conversation.set_model",
];
pub fn capabilities() -> Value {
    json!({"schema":"agent-runtime-capabilities/v1","platform":"browser-wasm","abi":{"major":1,"minor":3},"commands":COMMANDS,
        "events":{"schema":"agent-runtime-event/v1","transport":"worker"},
        "features":{"dynamic_rpc_additions":1,"workflow_edit_to_draft":true,"workflow_running_edit_guard":true,"host_tool_transport":1,"browser_resource_transactions":1},
        "unsupported":["native_filesystem","managed_process","native_grpc","ambient_environment","conversation_recovery","skill_content_update","cluster_update","image_path_import","agent_workflow_editing","workflow_edits_with_live_conversations"]})
}

pub struct Runtime {
    pub bridge: Arc<HostBridge>,
    framework: FrameworkState,
    bus: Arc<InMemoryEventBus>,
    projector: Arc<EventProjector>,
    workflows: Arc<WorkflowsModule>,
    sources: Arc<MemoryWorkflowSourceStore>,
    manager: tokio::sync::OnceCell<Arc<ConversationManager>>,
    started: AtomicBool,
    closing: AtomicBool,
    config: Mutex<Config>,
    mutation: tokio::sync::Mutex<()>,
    checkpoint: Mutex<Option<ResourceCheckpoint>>,
}
#[derive(Default)]
struct Config {
    language: String,
    skills: Vec<String>,
    clusters: BTreeMap<String, super::cluster::Cluster>,
    endpoints: BTreeMap<String, Value>,
}

impl Runtime {
    pub async fn create(options: Value) -> Result<Self> {
        if !options.is_object()
            || options
                .as_object()
                .unwrap()
                .keys()
                .any(|key| !matches!(key.as_str(), "schema" | "language"))
        {
            return Err("browser create options only support schema and language".into());
        }
        if options
            .get("schema")
            .and_then(Value::as_str)
            .is_some_and(|schema| schema != "agent-runtime-create-options/v1")
        {
            return Err("invalid create-options schema".into());
        }
        let language = ai_assistant::prompt_assets::set_language(
            options
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("zh"),
        )?;
        ai_assistant::persistence::set_auto_file_persistence_enabled(false);
        ai_assistant::skills::replace_skill_manager(SkillManager::new_with_embedded_system(
            "browser-skills",
        ))
        .await
        .map_err(err)?;
        let framework = FrameworkState::initialize().map_err(err)?;
        let bus = Arc::new(InMemoryEventBus::new());
        let projector = Arc::new(EventProjector::default());
        projector.install(&bus).await.map_err(err)?;
        let sources = Arc::new(MemoryWorkflowSourceStore::default());
        let workflows = Arc::new(
            WorkflowsModule::new_with_source_store(
                PathBuf::from("workflows"),
                bus.clone(),
                sources.clone(),
            )
            .map_err(err)?
            .with_runtime_registry(framework.registry()),
        );
        Ok(Self {
            bridge: Arc::new(HostBridge::default()),
            framework,
            bus,
            projector,
            workflows,
            sources,
            manager: tokio::sync::OnceCell::new(),
            started: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            config: Mutex::new(Config {
                language,
                ..Default::default()
            }),
            mutation: tokio::sync::Mutex::new(()),
            checkpoint: Mutex::new(None),
        })
    }
    pub async fn start(&self) -> Result<()> {
        self.ensure_open()?;
        let _gate = self.mutation.lock().await;
        if self.started.load(Ordering::Acquire) {
            return Ok(());
        }
        self.workflows
            .restore_local_sources(&self.framework.registry().runtime_tools())
            .await
            .map_err(err)?;
        self.manager
            .get_or_try_init(|| async {
                ConversationManager::new(self.framework.clone())
                    .await
                    .map(Arc::new)
                    .map_err(err)
            })
            .await?;
        self.started.store(true, Ordering::Release);
        Ok(())
    }
    fn ensure_open(&self) -> Result<()> {
        if self.closing.load(Ordering::Acquire) {
            Err("runtime is closed".into())
        } else {
            Ok(())
        }
    }
    fn manager(&self) -> Result<&Arc<ConversationManager>> {
        self.manager
            .get()
            .ok_or_else(|| "runtime has not been started".into())
    }
    pub async fn invoke(&self, request: Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let command_id = request
            .get("command_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("web-{}", uuid::Uuid::new_v4()));
        let command = request.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = request.get("payload").cloned().unwrap_or(json!({}));
        let result = if request.get("schema").and_then(Value::as_str)
            != Some("agent-runtime-command/v1")
            || !payload.is_object()
        {
            Err("invalid command envelope".into())
        } else {
            self.command(command, &payload, &command_id).await
        };
        match result {
            Ok(result) => {
                json!({"schema":"agent-runtime-result/v1","id":id,"command_id":command_id,"ok":true,"result":result})
            }
            Err(message) => {
                let kind = if message.starts_with("unsupported:") {
                    "unsupported"
                } else if message.contains("workflow_in_use:") {
                    "workflow_in_use"
                } else if message.starts_with("rpc_endpoint_exists:") {
                    "rpc_endpoint_exists"
                } else if message.contains("tool_name_conflict:") {
                    "rpc_tool_name_conflict"
                } else if message.starts_with("bad_state:") || message == "runtime is closed" {
                    "bad_state"
                } else {
                    "invalid_argument"
                };
                json!({"schema":"agent-runtime-result/v1","id":id,"command_id":command_id,"ok":false,"error":{"schema":"agent-runtime-error/v1","kind":kind,"message":message}})
            }
        }
    }
    async fn command(&self, command: &str, payload: &Value, command_id: &str) -> Result<Value> {
        self.ensure_open()?;
        if !COMMANDS.contains(&command) {
            return Err(format!("unsupported: {command}"));
        }
        let gate_needed =
            RESOURCE_COMMANDS.contains(&command) || command.starts_with("runtime.register_");
        let _gate = if gate_needed {
            Some(self.mutation.lock().await)
        } else {
            None
        };
        if self.checkpoint.lock().unwrap().is_some()
            && (gate_needed
                || command == "workflow.execute"
                || command == "runtime.get_workflow_node_definitions"
                || command == "workflow.list"
                || command == "workflow.validate")
        {
            return Err("bad_state: resource durability transaction is pending".into());
        }
        if RESOURCE_COMMANDS.contains(&command) {
            if self.manager.get().is_some() && !self.manager()?.list().await.is_empty() {
                return Err(
                    "workflow_in_use: close live browser conversations before editing resources"
                        .into(),
                );
            }
            let checkpoint = (
                self.workflows.checkpoint_catalog().map_err(err)?,
                self.sources.snapshot(),
            );
            *self.checkpoint.lock().unwrap() = Some(checkpoint);
            self.projector.begin_resource_commit();
        }
        let result = self.command_inner(command, payload, command_id).await;
        if result.is_err() && RESOURCE_COMMANDS.contains(&command) {
            self.finish_resource_commit(false)?;
        }
        result
    }
    async fn command_inner(&self, command: &str, p: &Value, command_id: &str) -> Result<Value> {
        match command {
            "runtime.register_resources" => {
                if self.started.load(Ordering::Acquire) {
                    return Err("resource registration is frozen after start".into());
                }
                let registration = p.get("registration").unwrap_or(p);
                if registration.get("schema").and_then(Value::as_str)
                    != Some("agent-runtime-resource-registration/v1")
                {
                    return Err("invalid resource registration schema".into());
                }
                if !self.config.lock().unwrap().clusters.is_empty() {
                    return Err("resource registration is frozen after cluster registration".into());
                }
                let mut skills = SkillManager::new_with_embedded_system("browser-skills");
                let sources: Vec<String> = serde_json::from_value(
                    registration
                        .pointer("/skills/sources")
                        .cloned()
                        .unwrap_or(json!([])),
                )
                .map_err(err)?;
                let mut names = std::collections::BTreeSet::new();
                for source in &sources {
                    let skill =
                        ai_assistant::skills::loader::parse_skill_content(source).map_err(err)?;
                    if skill.instructions.trim().is_empty()
                        || skill.metadata.system_layer
                        || skills.get(skill.name()).is_some()
                        || !names.insert(skill.name().to_owned())
                    {
                        return Err("Skill must have a unique name, nonempty body, and cannot replace system Skills".into());
                    }
                    skills.stage_candidate(skill);
                }
                let workflow_sources: Vec<String> = serde_json::from_value(
                    registration
                        .pointer("/workflows/sources")
                        .cloned()
                        .unwrap_or(json!([])),
                )
                .map_err(err)?;
                let mut documents = BTreeMap::new();
                for source in workflow_sources {
                    if corework::workflow::workflows::resource_api::resource_source_kind(&source)
                        .map_err(err)?
                        .is_none()
                    {
                        return Err("expected orbit.workflow.resource/v1".into());
                    }
                    let value = corework::workflow::logic::source::normalize_source(&source)
                        .map_err(err)?;
                    let id = required(&value, "id")?;
                    let key = PathBuf::from(format!("workflows/{id}.workflow.json"));
                    if documents.insert(key, source.into_bytes()).is_some() {
                        return Err("duplicate Workflow id".into());
                    }
                }
                ai_assistant::skills::replace_skill_manager(skills)
                    .await
                    .map_err(err)?;
                self.sources.replace(documents);
                self.config.lock().unwrap().skills = sources;
                Ok(json!({"registered":true}))
            }
            "runtime.register_llm" => {
                if self.started.load(Ordering::Acquire) {
                    return Err("LLM registration is frozen after start".into());
                }
                let registration = p.get("registration").unwrap_or(p).clone();
                let registration = serde_json::from_value(registration).map_err(err)?;
                let (_, config) =
                    llm_gateway::registration::build_llm_registry_and_config(registration)
                        .map_err(err)?;
                llm_gateway::registration::validate_current_model(
                    config.current_model_uid,
                    &config,
                )
                .map_err(err)?;
                llm_gateway::build_index_and_resolver(config);
                Ok(json!({"registered":true}))
            }
            "runtime.register_agent_cluster" => {
                if self.started.load(Ordering::Acquire) {
                    return Err("cluster registration is frozen after start".into());
                }
                let mut cluster: super::cluster::Cluster =
                    serde_json::from_value(p.get("registration").unwrap_or(p).clone())
                        .map_err(err)?;
                if self
                    .config
                    .lock()
                    .unwrap()
                    .clusters
                    .contains_key(&cluster.id)
                {
                    return Err("cluster already exists".into());
                }
                let catalog = ai_assistant::skills::systems::SKILL_MANAGER
                    .get()
                    .unwrap()
                    .read()
                    .await;
                cluster.validate(&catalog)?;
                drop(catalog);
                let id = cluster.id.clone();
                self.config
                    .lock()
                    .unwrap()
                    .clusters
                    .insert(id.clone(), cluster);
                Ok(json!({"registered":true,"id":id}))
            }
            "runtime.add_rpc_endpoint" => self.add_endpoint(p, command_id).await,
            "runtime.get_rpc_endpoint_definitions" => Ok(
                json!({"schema":"agent-runtime-rpc-endpoint-definitions/v1","catalog_revision":self.framework.registry().runtime_catalog_revision(),"endpoints":self.config.lock().unwrap().endpoints.values().cloned().collect::<Vec<_>>()}),
            ),
            "runtime.get_tool_definitions" => {
                let mut tools = BTreeMap::<String, Value>::new();
                for metadata in corework::system::SystemRegistry::list_ai_systems() {
                    tools.insert(
                        metadata.name.into(),
                        corework::tool_definitions::static_tool_definition(metadata),
                    );
                }
                for metadata in self.framework.registry().runtime_tools() {
                    tools.insert(
                        metadata.name.clone(),
                        corework::tool_definitions::runtime_tool_definition(&metadata),
                    );
                }
                Ok(
                    json!({"schema":"agent-runtime-tool-definitions/v1","catalog_revision":self.framework.registry().runtime_catalog_revision(),"catalog_scope":"runtime_registered","authorization_scope":"definitions_only","tools":tools.into_values().collect::<Vec<_>>()}),
                )
            }
            "runtime.get_workflow_node_definitions" => {
                let mut nodes =
                    corework::workflow::node_definitions::workflow_node_definition_values(
                        &self.framework.registry().runtime_tools(),
                        &ai_assistant::context::AssistantContext::all_registered_tool_names(),
                    );
                for definition in self
                    .workflows
                    .workflow_reference_definitions()
                    .map_err(err)?
                {
                    let mut node = definition["node"].clone();
                    node["category"] = json!("workflow/reference");
                    node["source"] = json!("workflow");
                    node["pure"] = json!(false);
                    node["name"] = node["node_type"].clone();
                    node["editor_callable"] = json!(true);
                    node["native_category"] = json!("Workflow Reference");
                    nodes.push(node);
                }
                Ok(
                    json!({"schema":"agent-runtime-workflow-node-definitions/v1","catalog_scope":"runtime_registered","nodes":nodes}),
                )
            }
            "runtime.set_current_model" => {
                let uid = p
                    .get("model_uid")
                    .and_then(Value::as_u64)
                    .and_then(|uid| u32::try_from(uid).ok())
                    .ok_or("invalid model_uid")?;
                if llm_gateway::key_store::get(uid).is_none() {
                    return Err("unknown model_uid".into());
                }
                llm_gateway::key_store::set_current(uid);
                Ok(json!({"model_uid":uid}))
            }
            "runtime.set_language" => {
                let language = ai_assistant::prompt_assets::set_language(required(p, "language")?)?;
                self.config.lock().unwrap().language = language.clone();
                Ok(json!({"language":language}))
            }
            "workflow.list" => {
                let kind = match p.get("kind").and_then(Value::as_str) {
                    None | Some("all") => None,
                    Some("draft") => Some(WorkflowResourceKind::Draft),
                    Some("registered") => Some(WorkflowResourceKind::Registered),
                    _ => return Err("invalid workflow kind".into()),
                };
                Ok(json!({"workflows":self.workflows.list_workflow_catalog(kind).map_err(err)?}))
            }
            "workflow.validate" | "workflow.register" => self
                .workflows
                .register_logic_resource(
                    required(p, "source")?,
                    &self.framework.registry().runtime_tools(),
                    command == "workflow.register",
                )
                .await
                .map_err(err),
            "workflow.remove" => self
                .workflows
                .delete_catalog_resource(required(p, "id")?, Some(revision(p)?))
                .await
                .map(|v| json!(v))
                .map_err(err),
            "workflow.promote" => self
                .workflows
                .register_draft_resource(required(p, "id")?, Some(revision(p)?), None)
                .await
                .map(|v| json!(v))
                .map_err(err),
            "workflow.execute" => {
                self.manager()?;
                let id = required(p, "id")?;
                let inputs: HashMap<String, Value> =
                    serde_json::from_value(p.get("inputs").cloned().unwrap_or(json!({})))
                        .map_err(err)?;
                let context = WorkflowExecutionContext {
                    conversation_id: p
                        .get("conversation_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    agent_id: p.get("agent_id").and_then(Value::as_str).map(str::to_owned),
                    turn_id: None,
                };
                if context.conversation_id.is_some() != context.agent_id.is_some() {
                    return Err("conversation_id and agent_id must be supplied together".into());
                }
                let view = self.workflows.read_workflow_resource(id).map_err(err)?;
                let _occupancy = self.workflows.reserve_catalog_execution(id).map_err(err)?;
                let started = corework::platform::time::Instant::now();
                let outcome = if view.summary.kind == WorkflowResourceKind::Registered {
                    self.workflows
                        .execute_registered_outcome_with_context(id, inputs, &context)
                        .await
                } else {
                    self.workflows
                        .execute_from_blueprint_outcome_with_context(
                            self.workflows.draft_blueprint(id).map_err(err)?,
                            inputs,
                            &context,
                        )
                        .await
                }
                .map_err(err)?;
                Ok(
                    json!({"code":if outcome.error.is_some(){-1}else{0},"run_id":outcome.report.trace.as_ref().map(|trace|&trace.run_id),"trace":outcome.report.to_ai(corework::workflow::execution::WorkflowToAiMode::Detailed,outcome.error.as_deref()),"result":{"outputs":outcome.report.outputs_json(),"duration_ms":started.elapsed().as_millis()}}),
                )
            }
            "conversation.spawn" => {
                let _guard = self.mutation.lock().await;
                if self.checkpoint.lock().unwrap().is_some() {
                    return Err("bad_state: resource durability transaction is pending".into());
                }
                self.spawn(p).await
            }
            "conversation.send_message" => {
                let parts = serde_json::from_value(p.get("parts").cloned().unwrap_or(json!([])))
                    .map_err(err)?;
                let admission = self
                    .manager()?
                    .send_message_with_parts_and_admission(
                        required(p, "conversation_id")?,
                        required(p, "content")?,
                        parts,
                        Some(command_id.into()),
                    )
                    .await
                    .map_err(err)?;
                Ok(
                    json!({"command_id":admission.command_id,"decision":match admission.decision {ai_assistant::admission::Decision::Accepted{..}=>"accepted",_=>"rejected"}}),
                )
            }
            "conversation.pause" => {
                self.manager()?
                    .request_pause(required(p, "conversation_id")?)
                    .await
                    .map_err(err)?;
                Ok(json!({"accepted":true}))
            }
            "conversation.close" => {
                let id = required(p, "conversation_id")?;
                let closed = self.manager()?.close(id).await.map_err(err)?;
                self.bus
                    .publish(BaseEvent::new(
                        "runtime:conversation_closed",
                        json!({"conversation_id":id}),
                    ))
                    .await
                    .map_err(err)?;
                Ok(json!({"closed":closed}))
            }
            "conversation.materialize" => {
                let id = required(p, "conversation_id")?;
                let ledger = self
                    .manager()?
                    .ledger(id, Default::default())
                    .await
                    .map_err(err)?;
                // The Worker cannot interleave a new append between this read
                // and the synchronous publication of its replacement snapshot.
                self.projector.emit(&BaseEvent::new(
                    "conversation.ledger_delta",
                    json!({"conversation_id":id,"kind":"replace","records":ledger}),
                ));
                self.manager()?
                    .publish_current_state(id)
                    .await
                    .map_err(err)?;
                Ok(
                    json!({"state":self.manager()?.frontend_state(id).await.map_err(err)?,"ledger":ledger}),
                )
            }
            "conversation.resolve_tool_permission" => {
                let decision =
                    serde_json::from_value(p.get("decision").cloned().ok_or("decision required")?)
                        .map_err(err)?;
                let resolved = self
                    .manager()?
                    .resolve_tool_permission(
                        required(p, "conversation_id")?,
                        required(p, "tool_call_id")?,
                        decision,
                    )
                    .await
                    .map_err(err)?;
                Ok(json!({"resolved":resolved}))
            }
            "conversation.set_model" => {
                let uid = p
                    .get("model_uid")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or("invalid model_uid")?;
                if llm_gateway::key_store::get(uid).is_none() {
                    return Err("unknown model_uid".into());
                }
                let value = self
                    .manager()?
                    .set_model_with_admission(
                        required(p, "conversation_id")?,
                        &uid.to_string(),
                        Some(command_id.into()),
                    )
                    .await
                    .map_err(err)?;
                Ok(json!({"command_id":value.command_id}))
            }
            _ => Err(format!("unsupported: {command}")),
        }
    }
    async fn add_endpoint(&self, p: &Value, command_id: &str) -> Result<Value> {
        if p.get("schema").and_then(Value::as_str) != Some("agent-runtime-rpc-endpoint-add/v1") {
            return Err("invalid RPC addition schema".into());
        }
        let endpoint = p.get("endpoint").ok_or("endpoint required")?;
        let id = required(endpoint, "id")?.to_string();
        if id.len() > 128
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
        {
            return Err("invalid endpoint id".into());
        }
        if !endpoint.is_object()
            || endpoint
                .as_object()
                .unwrap()
                .keys()
                .any(|key| !matches!(key.as_str(), "id" | "protocol" | "timeout_ms"))
        {
            return Err("browser endpoints only support id, protocol and timeout_ms".into());
        }
        if endpoint.get("protocol").and_then(Value::as_str) != Some("host") {
            return Err(
                "unsupported: browser RPC requires protocol=host and an SDK adapter".into(),
            );
        }
        if self.config.lock().unwrap().endpoints.contains_key(&id) {
            return Err(format!("rpc_endpoint_exists: {id}"));
        }
        let timeout = endpoint
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(30000);
        if !(1..=60000).contains(&timeout) {
            return Err("timeout_ms must be 1..60000".into());
        }
        let discovered = self
            .bridge
            .request("rpc.list_tools", endpoint.clone(), timeout)
            .await
            .map_err(err)?;
        let mut tools: Vec<RuntimeToolMetadata> = serde_json::from_value(
            discovered
                .get("tools")
                .cloned()
                .ok_or("ListTools tools required")?,
        )
        .map_err(err)?;
        if discovered.get("schema").and_then(Value::as_str) != Some("corework-agent-tool/v1")
            || tools.is_empty()
            || tools.len() > 1024
        {
            return Err("invalid host tool catalog".into());
        }
        let info = RpcEndpointInfo {
            endpoint_id: id.clone(),
            address: format!("host:{id}"),
            timeout_ms: timeout,
            to_ai_max_chars: 600,
        };
        for tool in &mut tools {
            if tool.endpoint_id.is_empty() {
                tool.endpoint_id = id.clone();
            }
            if tool.service.is_empty() {
                tool.service = corework::rpc_tool::RPC_TOOL_SERVICE_V1.into();
            }
            if tool.method.is_empty() {
                tool.method = "Execute".into();
            }
        }
        validate_runtime_tool_set_for_endpoint(&info, &tools).map_err(err)?;
        let endpoints = Arc::new(RpcEndpointRegistry::new());
        endpoints.insert(info).map_err(err)?;
        let table = Arc::new(RuntimeToolRegistry::new());
        for tool in &tools {
            table.insert(tool.clone()).map_err(err)?;
        }
        let entries = tools
            .iter()
            .map(|tool| {
                (
                    tool.clone(),
                    Arc::new(RpcStubSystem::new(
                        &tool.name,
                        endpoints.clone(),
                        table.clone(),
                        Arc::new(HostToolClient {
                            bridge: self.bridge.clone(),
                        }),
                    ))
                        as Arc<dyn corework::workflow::dynamic_node::DynamicExecute>,
                )
            })
            .collect();
        let _gate = self.mutation.lock().await;
        self.ensure_open()?;
        if self.checkpoint.lock().unwrap().is_some() {
            return Err("bad_state: resource durability transaction is pending".into());
        }
        if self.config.lock().unwrap().endpoints.contains_key(&id) {
            return Err(format!("rpc_endpoint_exists: {id}"));
        }
        let revision = self
            .framework
            .registry()
            .append_runtime_tools(entries)
            .map_err(err)?;
        let result = json!({"endpoint_id":id,"protocol":"host","catalog_revision":revision,"tool_count":tools.len(),"tool_names":tools.iter().map(|tool|&tool.name).collect::<Vec<_>>(),"persistence":"host_managed"});
        self.config
            .lock()
            .unwrap()
            .endpoints
            .insert(id, result.clone());
        self.bus
            .publish(BaseEvent::new(
                "runtime.rpc_catalog_changed",
                json!({"command_id":command_id,"change":result}),
            ))
            .await
            .map_err(err)?;
        Ok(result)
    }
    async fn spawn(&self, p: &Value) -> Result<Value> {
        let manager = self.manager()?;
        let cluster_id = required(p, "cluster_id")?;
        let (cluster, language) = {
            let config = self.config.lock().unwrap();
            (
                config
                    .clusters
                    .get(cluster_id)
                    .cloned()
                    .ok_or("unknown cluster_id")?,
                config.language.clone(),
            )
        };
        let primary = cluster
            .agents
            .iter()
            .find(|agent| agent.id == cluster.focus_agent_id)
            .ok_or("focus Agent missing")?;
        let id = format!("conv_{}", uuid::Uuid::new_v4());
        let mut options = ConversationOptions::new(&id, primary.config(&language));
        options.tool_permissions = match p.get("tool_permissions") {
            Some(policy) => serde_json::from_value(policy.clone()).map_err(err)?,
            None => cluster.permissions.clone(),
        };
        let info = manager
            .create_conversation(options, self.bus.clone())
            .await
            .map_err(err)?;
        let admission = async {
            manager
                .attach_shared_component(
                    &id,
                    Arc::new(ai_assistant::systems::workflow::WorkflowModuleHandle::new(
                        &self.workflows,
                    )),
                )
                .await
                .map_err(err)?;
            manager
                .attach_shared_component(
                    &id,
                    Arc::new(
                        ai_assistant::systems::workflow::WorkflowRuntimeToolCatalog::from_registry(
                            self.framework.registry(),
                        ),
                    ),
                )
                .await
                .map_err(err)?;
            manager
                .activate_skills(&id, &primary.skill_names())
                .await
                .map_err(err)?;
            for agent in cluster.agents.iter().filter(|agent| agent.id != primary.id) {
                manager
                    .register_agent(
                        &id,
                        agent.config(&language),
                        &agent.skill_names(),
                        cluster.max_thinking_rounds,
                        &BTreeMap::new(),
                    )
                    .await
                    .map_err(err)?;
            }
            use corework::cache::CacheExt;
            manager
                .default_agent_cache(&id)
                .await
                .map_err(err)?
                .set(
                    ai_assistant::context::keys::MAX_THINKING_ROUNDS,
                    &cluster.max_thinking_rounds,
                    None,
                )
                .await
                .map_err(err)?;
            Ok::<(), String>(())
        }
        .await;
        if let Err(error) = admission {
            let _ = manager.close(&id).await;
            return Err(error);
        }
        self.bus
            .publish(BaseEvent::new(
                "runtime:conversation_created",
                json!({"conversation_id":id,"cluster_id":cluster_id}),
            ))
            .await
            .map_err(err)?;
        manager.publish_current_state(&id).await.map_err(err)?;
        Ok(json!(info))
    }
    pub fn resource_snapshot(&self) -> Result<Value> {
        let workflows = self
            .sources
            .snapshot()
            .values()
            .map(|bytes| String::from_utf8(bytes.clone()).map_err(err))
            .collect::<Result<Vec<_>>>()?;
        let config = self.config.lock().unwrap();
        Ok(
            json!({"schema":"orbit-runtime-browser-resources/v1","workflow_sources":workflows,"skills":config.skills,"clusters":config.clusters.values().cloned().collect::<Vec<_>>()}),
        )
    }
    pub fn finish_resource_commit(&self, accepted: bool) -> Result<()> {
        if let Some((checkpoint, sources)) = self.checkpoint.lock().unwrap().take() {
            if !accepted {
                self.workflows.restore_catalog(checkpoint).map_err(err)?;
                self.sources.replace(sources);
            }
        }
        self.projector.finish_resource_commit(accepted);
        Ok(())
    }
    pub async fn shutdown(&self) -> Result<()> {
        if self.closing.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.bridge.cancel_all();
        if let Some(manager) = self.manager.get() {
            for info in manager.list().await {
                manager.close(&info.conversation_id).await.map_err(err)?;
            }
        }
        self.finish_resource_commit(false)?;
        Ok(())
    }
}
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn required<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("{name} is required"))
}
fn revision(p: &Value) -> Result<u64> {
    p.get("expected_revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| "expected_revision is required".into())
}
