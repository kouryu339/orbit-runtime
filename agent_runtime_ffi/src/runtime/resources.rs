use super::*;

pub(super) async fn build_resource_registry(
    mut registration: ResourceRegistration,
    base_dir: Option<&Path>,
) -> Result<RuntimeResourceRegistry, RuntimeError> {
    if registration.schema.trim().is_empty() {
        registration.schema = "agent-runtime-resource-registration/v1".to_string();
    }
    if registration.schema != "agent-runtime-resource-registration/v1" {
        return Err(RuntimeError::InvalidConfig(format!(
            "resource registration schema '{}' is not supported",
            registration.schema
        )));
    }
    validate_resource_id("resource registration id", &registration.id)?;

    let skills_root_dir = normalize_resource_path(base_dir, registration.skills.root_dir);
    let skill_manager = SkillManager::from_directory(&skills_root_dir)
        .await
        .map_err(|error| {
            RuntimeError::InvalidConfig(format!(
                "load skills from '{}' failed: {error}",
                skills_root_dir.display()
            ))
        })?;
    ai_assistant::skills::replace_skill_manager(skill_manager).await;

    let workflows_root_dir = registration
        .workflows
        .root_dir
        .map(|path| normalize_resource_path(base_dir, path));
    let workflow_registry = scan_parent_workflow_registry(workflows_root_dir.as_deref())?;
    let data_dir = registration
        .data
        .data_dir
        .map(|path| normalize_resource_path(base_dir, path));
    let logs_dir = registration
        .data
        .logs_dir
        .map(|path| normalize_resource_path(base_dir, path));
    let conversation_log_policy = registration.data.conversation_logs;
    validate_conversation_log_policy(&conversation_log_policy)?;
    let agent_profiles = build_agent_profile_registry(registration.agents.profiles)?;

    let mut rpc_pool = RuntimeRpcEndpointPool::default();
    for endpoint in registration.rpc_endpoints {
        rpc_pool.insert(resource_rpc_endpoint_to_runtime(endpoint, base_dir)?)?;
    }
    for profile in agent_profiles.values() {
        let Some(retrieval) = profile.retrieval.as_ref() else {
            continue;
        };
        let path = format!("agent profile '{}'.retrieval", profile.id);
        validate_retrieval_config(&path, retrieval)?;
        if !retrieval.enabled {
            continue;
        }
        let endpoint_id = retrieval
            .endpoint_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "{path}.endpoint_id must be configured when retrieval is enabled"
                ))
            })?;
        let endpoint = rpc_pool.get(endpoint_id).ok_or_else(|| {
            RuntimeError::InvalidConfig(format!(
                "{path} references unregistered endpoint '{}'",
                endpoint_id
            ))
        })?;
        if endpoint.protocol != "json-lines" {
            return Err(RuntimeError::InvalidConfig(format!(
                "{path} endpoint '{}' uses protocol '{}'; retrieval endpoints must use 'json-lines'",
                endpoint_id, endpoint.protocol
            )));
        }
    }

    Ok(RuntimeResourceRegistry {
        id: registration.id,
        skills_root_dir,
        builtin_system_skills: registration.skills.builtin_system,
        workflows_root_dir,
        workflow_registry_id: registration.workflows.registry_id,
        workflow_registry,
        data_dir,
        logs_dir,
        conversation_log_policy,
        agent_profiles,
        rpc_pool,
    })
}

fn build_agent_profile_registry(
    profiles: Vec<ResourceAgentProfileConfig>,
) -> Result<BTreeMap<String, ResourceAgentProfileConfig>, RuntimeError> {
    let mut registry = BTreeMap::new();
    for mut profile in profiles {
        let id = profile.id.trim().to_string();
        validate_resource_id("agent profile id", &id)?;
        if registry.contains_key(&id) {
            return Err(RuntimeError::InvalidConfig(format!(
                "agent profile '{}' is duplicated",
                id
            )));
        }
        profile.id = id.clone();
        profile.name = profile
            .name
            .take()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        profile.role = profile
            .role
            .take()
            .map(|role| role.trim().to_string())
            .filter(|role| !role.is_empty());
        profile.features = normalize_string_list(
            &format!("agent profile '{}'.features", id),
            profile.features,
        )?;
        registry.insert(id, profile);
    }
    Ok(registry)
}

fn scan_parent_workflow_registry(
    workflows_root_dir: Option<&Path>,
) -> Result<Vec<ParentWorkflowEntry>, RuntimeError> {
    let Some(workflows_root_dir) = workflows_root_dir else {
        return Ok(Vec::new());
    };
    let entries = match fs::read_dir(workflows_root_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(RuntimeError::InvalidConfig(format!(
                "scan workflow registry failed from '{}': {error}",
                workflows_root_dir.display()
            )))
        }
    };

    let mut registry: Vec<ParentWorkflowEntry> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !BlueprintJson::is_workflow_file_path(&path) {
            continue;
        }
        let blueprint = match BlueprintJson::from_workflow_file(&path) {
            Ok(blueprint) => blueprint,
            Err(error) => {
                return Err(RuntimeError::InvalidConfig(format!(
                    "parse workflow registry entry failed from '{}': {error}",
                    path.display()
                )))
            }
        };
        let entry = ParentWorkflowEntry::from_blueprint_path(path, &blueprint)
            .map_err(RuntimeError::InvalidConfig)?;
        registry.retain(|existing| {
            existing.file_name != entry.file_name
                && existing.name != entry.name
                && (entry.id.is_empty() || existing.id != entry.id)
        });
        registry.push(entry);
    }
    Ok(registry)
}

fn resource_rpc_endpoint_to_runtime(
    endpoint: ResourceRpcEndpointConfig,
    base_dir: Option<&Path>,
) -> Result<RpcToolEndpointConfig, RuntimeError> {
    let endpoint_id = endpoint.id.trim().to_string();
    let address = endpoint.endpoint.trim().to_string();
    validate_resource_id("rpc endpoint id", &endpoint_id)?;
    if address.is_empty() {
        return Err(RuntimeError::InvalidConfig(format!(
            "resource rpc endpoint '{}' endpoint must not be empty",
            endpoint_id
        )));
    }
    let protocol = endpoint.protocol.trim().to_string();
    if !matches!(protocol.as_str(), "grpc" | "json-lines") {
        return Err(RuntimeError::InvalidConfig(format!(
            "resource rpc endpoint '{}' protocol '{}' is not supported",
            endpoint_id, endpoint.protocol
        )));
    }
    Ok(RpcToolEndpointConfig {
        endpoint_id,
        address,
        protocol,
        launch: normalize_resource_launch_config(base_dir, endpoint.launch),
        timeout_ms: endpoint.timeout_ms,
        tools: Vec::new(),
    })
}

fn normalize_resource_launch_config(
    base_dir: Option<&Path>,
    launch: Option<RpcToolLaunchConfig>,
) -> Option<RpcToolLaunchConfig> {
    let mut launch = launch?;
    if let Some(program) = launch.program.take() {
        launch.program = Some(normalize_resource_command_path(base_dir, program));
    }
    if let Some(working_dir) = launch.working_dir.take() {
        launch.working_dir = Some(normalize_resource_path(base_dir, working_dir));
    }
    Some(launch)
}

fn normalize_resource_command_path(base_dir: Option<&Path>, path: PathBuf) -> PathBuf {
    if path.is_absolute() || path.components().count() <= 1 {
        path
    } else {
        normalize_resource_path(base_dir, path)
    }
}

pub(super) fn validate_resource_id(label: &str, value: &str) -> Result<(), RuntimeError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(RuntimeError::InvalidConfig(format!(
            "{label} must not be empty"
        )));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
    {
        return Err(RuntimeError::InvalidConfig(format!(
            "{label} '{}' contains unsupported characters",
            value
        )));
    }
    Ok(())
}

fn normalize_resource_path(base_dir: Option<&Path>, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base_dir
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .join(path)
    }
}

pub(super) async fn install_rpc_tools_from_config(
    configs: Vec<RpcToolEndpointConfig>,
) -> Result<(Vec<ManagedSidecar>, Vec<RuntimeToolMetadata>), RuntimeError> {
    if configs.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let framework = FrameworkState::initialize()
        .map_err(|e| RuntimeError::Internal(format!("initialize framework failed: {e}")))?;
    let system_registry = framework.registry();
    let endpoint_registry = Arc::new(RpcEndpointRegistry::new());
    let tool_registry = Arc::new(RuntimeToolRegistry::new());
    let discovery = GrpcRpcToolDiscoveryClient::default();
    let json_line_discovery = JsonLineRpcToolDiscoveryClient;
    let mut active_tools = Vec::new();
    let mut sidecar_children = Vec::new();

    for endpoint_config in configs {
        if endpoint_config.endpoint_id.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "rpc_tools endpoint_id must not be empty".to_string(),
            ));
        }
        if endpoint_config.address.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(format!(
                "rpc_tools endpoint '{}' address must not be empty",
                endpoint_config.endpoint_id
            )));
        }
        let endpoint_info = RpcEndpointInfo {
            endpoint_id: endpoint_config.endpoint_id.clone(),
            address: endpoint_config.address.clone(),
            timeout_ms: endpoint_config.timeout_ms,
        };
        let launch = endpoint_config
            .launch
            .clone()
            .unwrap_or_else(RpcToolLaunchConfig::default);
        if launch.kind == "process" {
            sidecar_children.push(launch_sidecar_process(&endpoint_config, &launch)?);
        }

        let (tools, client): (Vec<RuntimeToolMetadata>, Arc<dyn RpcToolClient>) =
            match endpoint_config.protocol.as_str() {
                "json-lines" => {
                    let tools = if endpoint_config.tools.is_empty() {
                        discover_json_line_tools_with_retry(
                            &json_line_discovery,
                            endpoint_info.clone(),
                            if launch.kind == "process" {
                                launch.startup_timeout_ms
                            } else {
                                0
                            },
                        )
                        .await?
                    } else {
                        endpoint_config.tools.clone()
                    };
                    (tools, Arc::new(JsonLineRpcToolClient))
                }
                "grpc" => {
                    if !endpoint_config.tools.is_empty() {
                        return Err(RuntimeError::InvalidConfig(format!(
                            "rpc_tools endpoint '{}' uses protocol=grpc; tool metadata must be served by AgentToolService.ListTools, not configured inline",
                            endpoint_config.endpoint_id
                        )));
                    }
                    (
                        discover_grpc_tools_with_retry(
                            &discovery,
                            endpoint_info.clone(),
                            if launch.kind == "process" {
                                launch.startup_timeout_ms
                            } else {
                                0
                            },
                        )
                        .await?,
                        Arc::new(GrpcRpcToolClient),
                    )
                }
                other => {
                    return Err(RuntimeError::Rpc(format!(
                        "rpc_tools endpoint '{}' protocol '{}' is not supported; use 'json-lines' or 'grpc'",
                        endpoint_config.endpoint_id, other
                    )));
                }
            };

        endpoint_registry
            .insert(endpoint_info.clone())
            .map_err(|e| RuntimeError::Rpc(e.to_string()))?;

        for mut tool in tools {
            if tool.name.trim().is_empty() {
                return Err(RuntimeError::InvalidConfig(format!(
                    "rpc_tools endpoint '{}' has a tool with empty name",
                    endpoint_config.endpoint_id
                )));
            }
            if tool.endpoint_id.trim().is_empty() {
                tool.endpoint_id = endpoint_config.endpoint_id.clone();
            }
            if tool.endpoint_id != endpoint_config.endpoint_id {
                return Err(RuntimeError::InvalidConfig(format!(
                    "rpc tool '{}' endpoint_id '{}' does not match enclosing endpoint '{}'",
                    tool.name, tool.endpoint_id, endpoint_config.endpoint_id
                )));
            }
            validate_runtime_tool_for_endpoint(&endpoint_info, &tool)
                .map_err(|e| RuntimeError::InvalidConfig(e.to_string()))?;

            tool_registry
                .insert(tool.clone())
                .map_err(|e| RuntimeError::Rpc(e.to_string()))?;
            ai_assistant::runtime_tools::register_runtime_tool(tool.clone());

            system_registry.register_dynamic(
                tool.name.clone(),
                Arc::new(RpcStubSystem::new(
                    tool.name.clone(),
                    Arc::clone(&endpoint_registry),
                    Arc::clone(&tool_registry),
                    client.clone(),
                )),
            );
            active_tools.push(tool);
        }
    }

    Ok((sidecar_children, active_tools))
}

pub(super) fn install_retrieval_system_from_config(
    retrieval_configs: &[RetrievalConfig],
    endpoints: &[RpcToolEndpointConfig],
) -> Result<(), RuntimeError> {
    let enabled = retrieval_configs
        .iter()
        .filter(|retrieval| retrieval.before_thinking_enabled())
        .collect::<Vec<_>>();
    if enabled.is_empty() {
        return Ok(());
    }
    let framework = FrameworkState::initialize()
        .map_err(|e| RuntimeError::Internal(format!("initialize framework failed: {e}")))?;
    let endpoint_map = Arc::new(
        endpoints
            .iter()
            .filter(|endpoint| endpoint.protocol == "json-lines")
            .map(|endpoint| (endpoint.endpoint_id.clone(), endpoint.clone()))
            .collect::<BTreeMap<_, _>>(),
    );
    let mut tool_names = BTreeSet::new();
    for retrieval in enabled {
        let endpoint_id = retrieval
            .endpoint_id
            .as_deref()
            .map(str::trim)
            .unwrap_or("");
        if !endpoint_map.contains_key(endpoint_id) {
            return Err(RuntimeError::InvalidConfig(format!(
                "agent retrieval endpoint '{}' is not a registered json-lines endpoint",
                endpoint_id
            )));
        }
        tool_names.insert(retrieval.tool_name.clone());
    }
    for tool_name in tool_names {
        ai_assistant::runtime_tools::register_runtime_tool(RuntimeToolMetadata {
            name: tool_name.clone(),
            display_name: "Retrieve Knowledge".to_string(),
            description: "Retrieve additional knowledge for the current agent from its configured retrieval endpoint. Use this for a more precise second-pass query when the automatically retrieved context is insufficient.".to_string(),
            tool_kind: "local".to_string(),
            parameters: vec![
                corework::rpc_tool::RuntimeAIParameter {
                    name: "query".to_string(),
                    param_type: "String".to_string(),
                    required: true,
                    default_value: None,
                    description: "Focused knowledge query.".to_string(),
                },
                corework::rpc_tool::RuntimeAIParameter {
                    name: "profiles".to_string(),
                    param_type: "Array".to_string(),
                    required: false,
                    default_value: None,
                    description: "Optional retrieval profiles; defaults to the agent configuration.".to_string(),
                },
                corework::rpc_tool::RuntimeAIParameter {
                    name: "top_k".to_string(),
                    param_type: "Integer".to_string(),
                    required: false,
                    default_value: None,
                    description: "Optional maximum result count.".to_string(),
                },
                corework::rpc_tool::RuntimeAIParameter {
                    name: "score_threshold".to_string(),
                    param_type: "Number".to_string(),
                    required: false,
                    default_value: None,
                    description: "Optional relevance threshold.".to_string(),
                },
            ],
            outputs: vec![corework::rpc_tool::RuntimeAIOutputField {
                name: "context".to_string(),
                field_type: "String".to_string(),
                description: "Knowledge context returned by the configured endpoint.".to_string(),
            }],
            destructive: false,
            readonly: true,
            idempotent: true,
            open_world: false,
            secret: false,
            required_capabilities: Vec::new(),
            endpoint_id: String::new(),
            service: "runtime.retrieval".to_string(),
            method: "Retrieve".to_string(),
        });
        framework.registry().register_dynamic(
            tool_name.clone(),
            Arc::new(JsonLineRetrievalSystem {
                tool_name,
                endpoints: Arc::clone(&endpoint_map),
            }),
        );
    }
    Ok(())
}

pub(super) async fn build_target_whitebox_contract(
    target_agent: &AgentSection,
    developer_brief: String,
    runtime_tools: &[RuntimeToolMetadata],
) -> Result<TargetWhiteboxContract, RuntimeError> {
    let skill_names = active_skill_names(target_agent);
    let mut role_skill = String::new();
    let mut feature_skills = Vec::new();
    let mut tool_names = Vec::new();
    if let Some(skill_manager) = ai_assistant::skills::systems::SKILL_MANAGER.get() {
        let refs: Vec<&str> = skill_names.iter().map(String::as_str).collect();
        let mut manager = skill_manager.write().await;
        manager
            .load_many(&refs)
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        for name in &skill_names {
            if let Some(skill) = manager.get(name) {
                if skill.metadata.is_role() {
                    role_skill = skill.instructions.clone();
                } else {
                    feature_skills.push(skill.instructions.clone());
                }
                for tool in &skill.metadata.tools {
                    if !tool_names.contains(tool) {
                        tool_names.push(tool.clone());
                    }
                }
            }
        }
    }
    let tools = tool_names
        .into_iter()
        .map(|name| {
            if let Some(metadata) = runtime_tools.iter().find(|tool| tool.name == name) {
                return TargetToolContract {
                    name,
                    description: metadata.description.clone(),
                    parameters_json: serde_json::to_value(&metadata.parameters)
                        .unwrap_or(Value::Null),
                };
            }
            if let Some(factory) =
                inventory::iter::<AISystemFactory>().find(|factory| factory.metadata.name == name)
            {
                return TargetToolContract {
                    name,
                    description: factory.metadata.description.to_string(),
                    parameters_json: json!(factory
                        .metadata
                        .parameters
                        .iter()
                        .map(|parameter| json!({
                            "name": parameter.name,
                            "type": parameter.param_type,
                            "required": parameter.required,
                            "default": parameter.default_value,
                            "description": parameter.description
                        }))
                        .collect::<Vec<_>>()),
                };
            }
            TargetToolContract {
                name,
                description: String::new(),
                parameters_json: Value::Array(Vec::new()),
            }
        })
        .collect();
    Ok(TargetWhiteboxContract {
        target_agent_id: target_agent.id.clone(),
        role_skill,
        feature_skills,
        tools,
        developer_brief,
    })
}

struct JsonLineRetrievalSystem {
    tool_name: String,
    endpoints: Arc<BTreeMap<String, RpcToolEndpointConfig>>,
}

#[async_trait]
impl DynamicExecute for JsonLineRetrievalSystem {
    async fn execute_dynamic(
        &self,
        mut input: HashMap<String, Value>,
        ctx: &corework::orchestration::Context,
    ) -> corework::error::Result<Value> {
        let retrieval = ctx
            .cache
            .get::<RetrievalConfig>(ai_assistant::context::keys::RETRIEVAL_CONFIG)
            .await?
            .ok_or_else(|| {
                corework::error::FrameworkError::InvalidOperation(
                    "retrieval is not configured for the current agent".to_string(),
                )
            })?;
        if !retrieval.enabled || retrieval.tool_name != self.tool_name {
            return Err(corework::error::FrameworkError::InvalidOperation(format!(
                "retrieval tool '{}' is not enabled for the current agent",
                self.tool_name
            )));
        }
        let endpoint_id = retrieval
            .endpoint_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                corework::error::FrameworkError::InvalidOperation(
                    "current agent retrieval endpoint_id is empty".to_string(),
                )
            })?;
        let endpoint = self.endpoints.get(endpoint_id).ok_or_else(|| {
            corework::error::FrameworkError::InvalidOperation(format!(
                "current agent retrieval endpoint '{}' is not registered",
                endpoint_id
            ))
        })?;
        input.entry("profiles".to_string()).or_insert_with(|| {
            Value::Array(
                retrieval
                    .profiles
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            )
        });
        if let Some(top_k) = retrieval.top_k {
            input
                .entry("top_k".to_string())
                .or_insert_with(|| Value::Number(top_k.into()));
        }
        if let Some(score_threshold) = retrieval.score_threshold {
            if let Some(number) = serde_json::Number::from_f64(score_threshold) {
                input
                    .entry("score_threshold".to_string())
                    .or_insert_with(|| Value::Number(number));
            }
        }
        let tool_name = self.tool_name.clone();
        let endpoint_id = endpoint.endpoint_id.clone();
        let address = endpoint.address.clone();
        let timeout_ms = endpoint.timeout_ms;
        let conversation_id = ctx.conversation_id.clone().unwrap_or_default();
        let request = json!({
            "type": "retrieval",
            "request": {
                "tool_name": tool_name,
                "conversation_id": conversation_id,
                "args_json": Value::Object(input.into_iter().collect()),
            }
        });

        tokio::task::spawn_blocking(move || {
            execute_jsonline_retrieval_request(&address, timeout_ms, &endpoint_id, request)
        })
        .await
        .map_err(|e| corework::error::FrameworkError::SystemError(e.to_string()))?
    }
}

fn execute_jsonline_retrieval_request(
    address: &str,
    timeout_ms: u64,
    endpoint_id: &str,
    request: Value,
) -> corework::error::Result<Value> {
    let timeout = Duration::from_millis(timeout_ms.max(1));
    let mut stream = TcpStream::connect(address).map_err(|e| {
        corework::error::FrameworkError::SystemError(format!(
            "retrieval endpoint '{}' connect {} failed: {}",
            endpoint_id, address, e
        ))
    })?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    let line = serde_json::to_string(&request)
        .map_err(corework::error::FrameworkError::SerializationError)?;
    stream.write_all(line.as_bytes()).map_err(|e| {
        corework::error::FrameworkError::SystemError(format!(
            "retrieval endpoint '{}' write failed: {}",
            endpoint_id, e
        ))
    })?;
    stream.write_all(b"\n").map_err(|e| {
        corework::error::FrameworkError::SystemError(format!(
            "retrieval endpoint '{}' write newline failed: {}",
            endpoint_id, e
        ))
    })?;
    let _ = stream.shutdown(Shutdown::Write);

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    reader.read_line(&mut response_line).map_err(|e| {
        corework::error::FrameworkError::SystemError(format!(
            "retrieval endpoint '{}' read failed: {}",
            endpoint_id, e
        ))
    })?;
    let response: Value = serde_json::from_str(response_line.trim()).map_err(|e| {
        corework::error::FrameworkError::SystemError(format!(
            "retrieval endpoint '{}' returned invalid json: {}",
            endpoint_id, e
        ))
    })?;
    if response.get("type").and_then(Value::as_str) != Some("retrieval_output") {
        return Err(corework::error::FrameworkError::SystemError(format!(
            "retrieval endpoint '{}' returned unexpected message type",
            endpoint_id
        )));
    }
    Ok(response.get("output").cloned().unwrap_or(Value::Null))
}

pub(super) fn install_workflow_runtime_from_config(
    auto_load_dir: Option<PathBuf>,
) -> Result<Arc<WorkflowsModule>, RuntimeError> {
    let dir = auto_load_dir.unwrap_or_else(|| PathBuf::from("workflows"));
    let module = WorkflowsModule::new(dir.clone())
        .map_err(|e| RuntimeError::Internal(format!("initialize workflow runtime failed: {e}")))?;
    let count = module
        .scan_local_dir()
        .map_err(|e| RuntimeError::Internal(format!("read workflow registry failed: {e}")))?;
    tracing::info!(
        "workflow runtime initialized from {} with {} workflows",
        dir.display(),
        count
    );
    Ok(Arc::new(module))
}
fn launch_sidecar_process(
    endpoint_config: &RpcToolEndpointConfig,
    launch: &RpcToolLaunchConfig,
) -> Result<ManagedSidecar, RuntimeError> {
    let program = launch.program.as_ref().ok_or_else(|| {
        RuntimeError::InvalidConfig(format!(
            "rpc_tools endpoint '{}' launch.program is required when launch.kind=process",
            endpoint_config.endpoint_id
        ))
    })?;

    let mut command = Command::new(program);
    command
        .args(&launch.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(working_dir) = &launch.working_dir {
        command.current_dir(working_dir);
    }
    for (key, value) in &launch.env {
        command.env(key, value);
    }

    let child = command.spawn().map_err(|e| {
        RuntimeError::Rpc(format!(
            "start rpc_tools endpoint '{}' sidecar failed: {e}",
            endpoint_config.endpoint_id
        ))
    })?;

    Ok(ManagedSidecar {
        child,
        shutdown_timeout_ms: launch.shutdown_timeout_ms,
    })
}

async fn discover_grpc_tools_with_retry(
    discovery: &GrpcRpcToolDiscoveryClient,
    endpoint_info: RpcEndpointInfo,
    startup_timeout_ms: u64,
) -> Result<Vec<RuntimeToolMetadata>, RuntimeError> {
    if startup_timeout_ms == 0 {
        return discovery
            .list_tools(endpoint_info)
            .await
            .map_err(|e| RuntimeError::Rpc(e.to_string()));
    }

    let deadline = Instant::now() + Duration::from_millis(startup_timeout_ms);
    loop {
        let error = match discovery.list_tools(endpoint_info.clone()).await {
            Ok(tools) => return Ok(tools),
            Err(error) => error.to_string(),
        };

        if Instant::now() >= deadline {
            return Err(RuntimeError::Rpc(format!(
                "rpc_tools endpoint '{}' did not become ready within {}ms: {}",
                endpoint_info.endpoint_id, startup_timeout_ms, error
            )));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

async fn discover_json_line_tools_with_retry(
    discovery: &JsonLineRpcToolDiscoveryClient,
    endpoint_info: RpcEndpointInfo,
    startup_timeout_ms: u64,
) -> Result<Vec<RuntimeToolMetadata>, RuntimeError> {
    if startup_timeout_ms == 0 {
        return discovery
            .list_tools(endpoint_info)
            .await
            .map_err(|e| RuntimeError::Rpc(e.to_string()));
    }

    let deadline = Instant::now() + Duration::from_millis(startup_timeout_ms);
    loop {
        let error = match discovery.list_tools(endpoint_info.clone()).await {
            Ok(tools) => return Ok(tools),
            Err(error) => error.to_string(),
        };
        if Instant::now() >= deadline {
            return Err(RuntimeError::Rpc(format!(
                "rpc_tools endpoint '{}' did not become ready within {}ms: {}",
                endpoint_info.endpoint_id, startup_timeout_ms, error
            )));
        }
        sleep(Duration::from_millis(100)).await;
    }
}
