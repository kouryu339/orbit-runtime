use super::*;
use std::sync::{MutexGuard, OnceLock};

static RUNTIME_START_TEST_LOCK: OnceLock<StdMutex<()>> = OnceLock::new();

fn runtime_start_test_guard() -> MutexGuard<'static, ()> {
    RUNTIME_START_TEST_LOCK
        .get_or_init(|| StdMutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unique_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "agent-runtime-ffi-{name}-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_role_skill(skills_dir: &Path, name: &str, kind: &str, body: &str) {
    let skill_dir = skills_dir.join("role").join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: test role\nkind: {kind}\n---\n\n{body}\n"),
    )
    .unwrap();
}

fn minimal_runtime_create_options(root: &Path) -> String {
    json!({
        "schema": "agent-runtime-create-options/v1",
        "log_level": "info",
        "language": "zh",
        "restore_policy": "strict",
        "data_dir": root.join("data")
    })
    .to_string()
}

fn register_test_llm(facade: &mut RuntimeFacade, model_uid: u32) {
    // Built-in studio clusters currently refer to model UID 1001, so tests that
    // start the runtime register it alongside their scenario-specific model.
    let mut enabled_models = vec![json!({
        "uid": model_uid,
        "model_id": "test-model"
    })];
    if model_uid != 1001 {
        enabled_models.push(json!({
            "uid": 1001,
            "model_id": "test-model"
        }));
    }
    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "test-llm",
                "current_model_uid": model_uid,
                "providers": [{
                    "id": 1,
                    "name": "test-provider",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "api_paradigm": "openai_chat_completions",
                    "enabled_models": enabled_models
                }]
            })
            .to_string(),
        )
        .unwrap();
}

#[test]
fn runtime_state_backend_defaults_to_map_and_supports_ecs_opt_in() {
    let default_config: RuntimeConfig = serde_json::from_value(json!({})).unwrap();
    assert_eq!(
        default_config.runtime.state_backend,
        RuntimeStateBackendConfig::Map
    );

    let ecs_config: RuntimeConfig = serde_json::from_value(json!({
        "runtime": { "state_backend": "ecs" }
    }))
    .unwrap();
    assert_eq!(
        ecs_config.runtime.state_backend,
        RuntimeStateBackendConfig::Ecs
    );
}

#[test]
fn conversation_log_policy_rejects_invalid_limits() {
    let error = validate_conversation_log_policy(&ConversationLogPolicy {
        max_files_per_cluster: 0,
        ..ConversationLogPolicy::default()
    })
    .unwrap_err();
    assert!(error.to_string().contains("max_files_per_cluster"));

    let error = validate_conversation_log_policy(&ConversationLogPolicy {
        max_file_bytes: 1_023,
        ..ConversationLogPolicy::default()
    })
    .unwrap_err();
    assert!(error.to_string().contains("max_file_bytes"));
}

#[test]
fn conversation_log_policy_prunes_cluster_files_and_enforces_size_limit() {
    let root = unique_test_dir("conversation-log-policy");
    let policy = ConversationLogPolicy {
        retention_days: 0,
        max_files_per_cluster: 2,
        max_file_bytes: 1_024,
    };
    let cluster_id = "cluster/with:unsafe";

    for index in 0..3 {
        let path = create_conversation_log_path(
            Some(&root),
            cluster_id,
            &format!("conversation-{index}"),
            chrono::Utc::now(),
            &policy,
        )
        .expect("conversation log path");
        append_conversation_log_path(
            Some(&path),
            "runtime-test",
            cluster_id,
            &format!("conversation-{index}"),
            "conversation_created",
            Value::Object(Default::default()),
            policy.max_file_bytes,
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let cluster_dir = root.join("cluster_with_unsafe");
    let files = fs::read_dir(&cluster_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("log"))
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 2);

    let path = files[0].path();
    let before = fs::metadata(&path).unwrap().len();
    append_conversation_log_path(
        Some(&path),
        "runtime-test",
        cluster_id,
        "conversation-size",
        "oversized",
        json!({ "summary": "x".repeat(2_048) }),
        policy.max_file_bytes,
    );
    assert_eq!(fs::metadata(&path).unwrap().len(), before);
    let _ = fs::remove_dir_all(root);
}

fn test_conversation_info(conversation_id: &str) -> ConversationInfo {
    ConversationInfo {
        conversation_id: conversation_id.to_string(),
        tenant_id: Some("tenant-a".to_string()),
        user_id: Some("user-a".to_string()),
        scope_id: format!("tenant:tenant-a:conversation:{conversation_id}"),
        created_at: chrono::Utc::now(),
    }
}

#[test]
fn ai_auth_context_accepts_runtime_header_map() {
    let headers = parse_ai_auth_context_headers(
        &json!({
            "headers": {
                "Authorization": "Bearer token-a",
                "X-App-Meta": { "tenant": "sunwoo", "role": "admin" },
                "X-Empty": null
            }
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer token-a")
    );
    assert_eq!(
        headers.get("X-App-Meta").map(String::as_str),
        Some(r#"{"role":"admin","tenant":"sunwoo"}"#)
    );
    assert!(!headers.contains_key("X-Empty"));
}

#[test]
fn ai_auth_context_accepts_legacy_access_token_shape() {
    let headers = parse_ai_auth_context_headers(
        &json!({
            "access_token": "token-b",
            "app_meta": { "tenant": "sunwoo" }
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer token-b")
    );
    assert_eq!(
        headers.get("X-App-Meta").map(String::as_str),
        Some(r#"{"tenant":"sunwoo"}"#)
    );
}

#[test]
fn ai_auth_context_accepts_top_level_headers() {
    let headers = parse_ai_auth_context_headers(
        &json!({
            "Authorization": "Bearer token-c",
            "X-App-Meta": "desktop"
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer token-c")
    );
    assert_eq!(
        headers.get("X-App-Meta").map(String::as_str),
        Some("desktop")
    );
}

#[test]
fn project_event_adds_stable_host_envelope() {
    let event = BaseEvent::new(
        ai_assistant::events::types::ASKING,
        json!({
            "content": "hello",
            "turn_id": 7
        }),
    );

    let envelope = project_event(&event, 42);

    assert_eq!(envelope["schema"], "agent-runtime-event/v1");
    assert_eq!(envelope["event_seq"], 42);
    assert_eq!(envelope["type"], "assistant_message");
    assert_eq!(envelope["source"], ai_assistant::events::types::ASKING);
    assert_eq!(envelope["event_id"], event.event_id);
    assert_eq!(envelope["payload"]["content"], "hello");
    assert_eq!(envelope["payload"]["turn_id"], 7);
}

#[test]
fn project_pause_requested_uses_host_control_event_name() {
    let event = BaseEvent::new(
        ai_assistant::events::types::AGENT_PAUSE_REQUESTED,
        json!({
            "agent_id": "boss",
            "agent_name": "Boss"
        }),
    );

    let envelope = project_event(&event, 7);

    assert_eq!(envelope["type"], "pause_requested");
    assert_eq!(
        envelope["source"],
        ai_assistant::events::types::AGENT_PAUSE_REQUESTED
    );
    assert_eq!(envelope["payload"]["agent_id"], "boss");
}

#[test]
fn project_focus_status_as_ui_snapshot_changed() {
    let event = BaseEvent::new(
        ai_assistant::events::types::FOCUS_STATUS_CHANGED,
        json!({
            "agent_id": "boss",
            "agent_name": "Boss",
            "focused_agent_id": "boss",
            "status": { "kind": "working", "text": "姝ｅ湪澶勭悊" },
            "interaction": {
                "input_enabled": false,
                "send_enabled": false,
                "pause_visible": true,
                "pause_enabled": true,
                "pause_label": "鏆傚仠",
                "busy": true
            }
        }),
    );

    let envelope = project_event(&event, 8);

    assert_eq!(envelope["type"], "ui_snapshot_changed");
    assert_eq!(
        envelope["source"],
        ai_assistant::events::types::FOCUS_STATUS_CHANGED
    );
    assert_eq!(envelope["payload"]["status"]["kind"], "working");
    assert_eq!(envelope["payload"]["interaction"]["pause_enabled"], true);
}

#[tokio::test]
async fn project_ledger_record_append_as_conversation_delta() {
    let event = BaseEvent::new(
        ai_assistant::events::types::CONVERSATION_LEDGER_DELTA,
        json!({
            "schema": "agent-runtime-ledger-delta/v1",
            "op": "append",
            "record_id": 12,
            "conversation_id": "conv-ledger",
            "record": {
                "record_id": 12,
                "conversation_id": "conv-ledger",
                "agent_id": "agent-a",
                "agent_name": "Agent A",
                "role": "assistant",
                "content": "persist me incrementally",
                "metadata": {},
                "created_at": "2026-06-23T10:00:00+08:00"
            }
        }),
    );

    let projector = HostEventProjector::default();
    let envelope = projector.project(&event).await.unwrap();

    assert_eq!(envelope["type"], "conversation.ledger_delta");
    assert_eq!(
        envelope["source"],
        ai_assistant::events::types::CONVERSATION_LEDGER_DELTA
    );
    assert_eq!(envelope["conversation_id"], "conv-ledger");
    assert_eq!(envelope["conversation_event_seq"], 1);
    assert_eq!(
        envelope["payload"]["schema"],
        "agent-runtime-ledger-delta/v1"
    );
    assert_eq!(envelope["payload"]["op"], "append");
    assert_eq!(envelope["payload"]["record_id"], 12);
    assert_eq!(envelope["payload"]["conversation_id"], "conv-ledger");
    assert_eq!(
        envelope["payload"]["record"]["content"],
        "persist me incrementally"
    );
}

#[tokio::test]
async fn project_agent_task_event_as_state_delta() {
    let event = BaseEvent::new(
        ai_assistant::events::types::CONVERSATION_STATE_DELTA,
        json!({
            "schema": "agent-runtime-state-delta/v1",
            "op": "agent_task.upsert",
            "conversation_id": "conv-state",
            "task_id": "task-1",
            "task": {
                "task_id": "task-1",
                "title": "Check order",
                "objective": "Verify external order state",
                "delegator_agent_id": "boss",
                "delegator_agent_name": "Boss",
                "assignee_agent_id": "worker",
                "assignee_agent_name": "Worker",
                "status": "running",
                "created_at": "2026-06-23T10:00:00+08:00",
                "updated_at": "2026-06-23T10:01:00+08:00"
            }
        }),
    );

    let projector = HostEventProjector::default();
    let envelope = projector.project(&event).await.unwrap();

    assert_eq!(envelope["type"], "conversation.state_delta");
    assert_eq!(envelope["conversation_id"], "conv-state");
    assert_eq!(
        envelope["payload"]["schema"],
        "agent-runtime-state-delta/v1"
    );
    assert_eq!(envelope["payload"]["op"], "agent_task.upsert");
    assert_eq!(envelope["payload"]["task_id"], "task-1");
    assert_eq!(envelope["payload"]["task"]["assignee_agent_id"], "worker");
}

#[tokio::test]
async fn project_runtime_dynamic_snapshot_as_state_delta() {
    let event = BaseEvent::new(
        ai_assistant::events::types::CONVERSATION_STATE_DELTA,
        json!({
            "schema": "agent-runtime-state-delta/v1",
            "op": "dynamic_snapshot.set",
            "conversation_id": "conv-state",
            "agent_id": "worker",
            "field": "order_context",
            "text": "current host snapshot",
            "host_owned": true,
            "stale_after_restore": true
        }),
    );

    let projector = HostEventProjector::default();
    let envelope = projector.project(&event).await.unwrap();

    assert_eq!(envelope["type"], "conversation.state_delta");
    assert_eq!(envelope["conversation_id"], "conv-state");
    assert_eq!(envelope["payload"]["op"], "dynamic_snapshot.set");
    assert_eq!(envelope["payload"]["host_owned"], true);
    assert_eq!(envelope["payload"]["stale_after_restore"], true);
}

#[tokio::test]
async fn host_event_projector_assigns_monotonic_event_seq() {
    let projector = HostEventProjector::default();
    let first = BaseEvent::new(ai_assistant::events::types::TOOL_START, json!({}));
    let second = BaseEvent::new(ai_assistant::events::types::TOOL_END, json!({}));

    let first_envelope = projector.project(&first).await.unwrap();
    let second_envelope = projector.project(&second).await.unwrap();

    assert_eq!(first_envelope["event_seq"], 1);
    assert_eq!(second_envelope["event_seq"], 2);
}

#[tokio::test]
async fn conversation_projectors_share_global_event_seq() {
    let sequence_backend: Arc<dyn RuntimeSequenceBackend> =
        Arc::new(LocalRuntimeSequenceBackend::default());
    let metadata = RuntimeEventMetadata::default();
    let conv_a = HostEventProjector::for_conversation(
        Arc::clone(&sequence_backend),
        "conv_a",
        metadata.clone(),
    );
    let conv_b =
        HostEventProjector::for_conversation(Arc::clone(&sequence_backend), "conv_b", metadata);
    let event = BaseEvent::new(ai_assistant::events::types::TURN_START, json!({}));

    let a_first = conv_a.project(&event).await.unwrap();
    let b_first = conv_b.project(&event).await.unwrap();
    let a_second = conv_a.project(&event).await.unwrap();

    assert_eq!(a_first["event_seq"], 1);
    assert_eq!(b_first["event_seq"], 2);
    assert_eq!(a_second["event_seq"], 3);
    assert_eq!(a_first["conversation_id"], "conv_a");
    assert_eq!(b_first["conversation_id"], "conv_b");
    assert_eq!(a_first["conversation_event_seq"], 1);
    assert_eq!(b_first["conversation_event_seq"], 1);
    assert_eq!(a_second["conversation_event_seq"], 2);
}

#[tokio::test]
async fn local_coordination_backend_enforces_owner_lease() {
    let backend = LocalRuntimeCoordinationBackend::default();

    assert!(backend
        .acquire_lease("conversation:1:turn", "pod-a", 30_000)
        .await
        .unwrap());
    assert!(!backend
        .acquire_lease("conversation:1:turn", "pod-b", 30_000)
        .await
        .unwrap());
    assert!(!backend
        .renew_lease("conversation:1:turn", "pod-b", 30_000)
        .await
        .unwrap());

    backend
        .release_lease("conversation:1:turn", "pod-b")
        .await
        .unwrap();
    assert!(!backend
        .acquire_lease("conversation:1:turn", "pod-b", 30_000)
        .await
        .unwrap());

    backend
        .release_lease("conversation:1:turn", "pod-a")
        .await
        .unwrap();
    assert!(backend
        .acquire_lease("conversation:1:turn", "pod-b", 30_000)
        .await
        .unwrap());
}

#[test]
fn lease_renew_interval_clamps_to_ttl_and_has_fallback() {
    assert_eq!(
        lease_renew_interval(30_000, 10_000),
        Duration::from_millis(10_000)
    );
    assert_eq!(
        lease_renew_interval(30_000, 60_000),
        Duration::from_millis(30_000)
    );
    assert_eq!(lease_renew_interval(9_000, 0), Duration::from_millis(3_000));
    assert_eq!(lease_renew_interval(0, 0), Duration::from_millis(1));
}

#[tokio::test]
async fn lease_renewer_extends_local_lease_until_stopped() {
    let backend: Arc<dyn RuntimeCoordinationBackend> =
        Arc::new(LocalRuntimeCoordinationBackend::default());
    let key = "conversation:renew:turn".to_string();
    let owner = "pod-a".to_string();

    assert!(backend.acquire_lease(&key, &owner, 15).await.unwrap());
    let (stop_tx, stop_rx) = watch::channel(false);
    let renewer = tokio::spawn(run_lease_renewer(
        Arc::clone(&backend),
        key.clone(),
        owner.clone(),
        50,
        Duration::from_millis(5),
        stop_rx,
    ));

    sleep(Duration::from_millis(30)).await;
    assert!(!backend.acquire_lease(&key, "pod-b", 50).await.unwrap());

    let _ = stop_tx.send(true);
    let _ = renewer.await;
    backend.release_lease(&key, &owner).await.unwrap();
    assert!(backend.acquire_lease(&key, "pod-b", 50).await.unwrap());
}

#[tokio::test]
async fn conversation_owner_lease_prevents_same_id_active_runtime() {
    let backend: Arc<dyn RuntimeCoordinationBackend> =
        Arc::new(LocalRuntimeCoordinationBackend::default());
    let lease_a = acquire_conversation_owner_lease(
        Arc::clone(&backend),
        "support".to_string(),
        "conv-owner".to_string(),
        "pod-a".to_string(),
        50,
        Duration::from_millis(5),
    )
    .await
    .unwrap()
    .unwrap();

    let lease_b = acquire_conversation_owner_lease(
        Arc::clone(&backend),
        "support".to_string(),
        "conv-owner".to_string(),
        "pod-b".to_string(),
        50,
        Duration::from_millis(5),
    )
    .await
    .unwrap();
    assert!(lease_b.is_none());

    sleep(Duration::from_millis(30)).await;
    assert!(!backend
        .acquire_lease(
            &conversation_owner_lease_key("support", "conv-owner"),
            "pod-b",
            50
        )
        .await
        .unwrap());

    lease_a
        .stop_and_release(Arc::clone(&backend))
        .await
        .unwrap();
    assert!(backend
        .acquire_lease(
            &conversation_owner_lease_key("support", "conv-owner"),
            "pod-b",
            50
        )
        .await
        .unwrap());
}

#[tokio::test]
async fn runtime_state_store_records_conversation_catalog() {
    let state_store: Arc<dyn RuntimeStateStore> = Arc::new(LocalRuntimeStateStore::default());
    let coordination_backend: Arc<dyn RuntimeCoordinationBackend> =
        Arc::new(LocalRuntimeCoordinationBackend::default());
    let first = test_conversation_info("conv-a");
    let second = test_conversation_info("conv-b");

    record_conversation_created(
        Arc::clone(&state_store),
        Arc::clone(&coordination_backend),
        "support".to_string(),
        "pod-a".to_string(),
        30_000,
        first.clone(),
    )
    .await
    .unwrap();
    record_conversation_created(
        Arc::clone(&state_store),
        Arc::clone(&coordination_backend),
        "support".to_string(),
        "pod-a".to_string(),
        30_000,
        second.clone(),
    )
    .await
    .unwrap();

    let index = load_conversation_index(&state_store, "support")
        .await
        .unwrap();
    assert_eq!(index.len(), 2);
    assert_eq!(index[0].conversation_id, "conv-a");
    assert_eq!(index[1].conversation_id, "conv-b");

    let metadata = state_store
        .get_json(&conversation_metadata_key("support", "conv-a"))
        .await
        .unwrap()
        .unwrap();
    let metadata: ConversationInfo = serde_json::from_value(metadata).unwrap();
    assert_eq!(metadata.conversation_id, first.conversation_id);
    assert_eq!(metadata.scope_id, first.scope_id);

    record_conversation_closed(
        Arc::clone(&state_store),
        Arc::clone(&coordination_backend),
        "support".to_string(),
        "pod-a".to_string(),
        30_000,
        "conv-a".to_string(),
    )
    .await
    .unwrap();

    let index = load_conversation_index(&state_store, "support")
        .await
        .unwrap();
    assert_eq!(index.len(), 1);
    assert_eq!(index[0].conversation_id, "conv-b");
    assert!(state_store
        .get_json(&conversation_metadata_key("support", "conv-a"))
        .await
        .unwrap()
        .is_none());
}

#[test]
fn runtime_section_accepts_runtime_identity_config() {
    let runtime: RuntimeSection = serde_json::from_value(json!({
        "cluster_id": "support",
        "runtime_profile_id": "support-v2",
        "cluster_fingerprint": "sha256:test",
        "runtime_instance_id": "pod-1",
        "persistence": { "mode": "host_managed" },
        "restore_policy": "strict"
    }))
    .unwrap();

    assert_eq!(runtime.cluster_id, "support");
    assert_eq!(runtime.runtime_profile_id, "support-v2");
    assert_eq!(runtime.persistence.mode(), PersistenceMode::HostManaged);
    assert!(!runtime.persistence.auto_file_persistence_enabled());
}

#[test]
fn agent_section_accepts_agent_level_retrieval_config() {
    let agent: AgentSection = serde_json::from_value(json!({
        "id": "support-agent",
        "retrieval": {
            "enabled": true,
            "mode": "before_thinking",
            "trigger": "first_thinking_per_user_turn",
            "tool_name": "RagRetrieve",
            "profiles": ["order_admin_policy"],
            "top_k": 3,
            "score_threshold": 0.42,
            "fail_policy": "soft",
            "inject_as": "dynamic_context"
        }
    }))
    .unwrap();

    let retrieval = agent.retrieval.unwrap();
    assert!(retrieval.enabled);
    assert_eq!(retrieval.tool_name, "RagRetrieve");
    assert_eq!(retrieval.profiles, vec!["order_admin_policy".to_string()]);
    assert_eq!(retrieval.top_k, Some(3));
}

#[test]
fn resource_registration_rejects_external_prompts_field() {
    let error = serde_json::from_value::<ResourceRegistration>(json!({
        "schema": "agent-runtime-resource-registration/v1",
        "id": "default-resources",
        "skills": { "root_dir": "./skills" },
        "prompts": { "root_dir": "./prompts" }
    }))
    .unwrap_err();

    assert!(error.to_string().contains("prompts"));
}

#[test]
fn resource_registration_initializes_skill_manager_and_rpc_pool() {
    let root = unique_test_dir("resource-registration");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );

    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                },
                "data": {
                    "data_dir": root.join("data"),
                    "logs_dir": root.join("logs")
                },
                "agents": {
                    "profiles": [{
                        "id": "background.researcher",
                        "name": "Background Researcher",
                        "role": "browser_operator",
                        "features": ["delegated-reporting"]
                    }]
                },
                "rpc_endpoints": [
                    {
                        "id": "browser-tools",
                        "protocol": "grpc",
                        "endpoint": "http://127.0.0.1:50051"
                    },
                    {
                        "id": "retrieval",
                        "protocol": "json-lines",
                        "endpoint": "127.0.0.1:47071"
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();

    let resources = facade.registries.resources.as_ref().unwrap();
    assert_eq!(resources.id, "default-resources");
    assert!(resources.skills_root_dir.ends_with("skills"));
    assert_eq!(
        resources
            .rpc_pool
            .get("browser-tools")
            .map(|endpoint| endpoint.protocol.as_str()),
        Some("grpc")
    );
    assert_eq!(
        resources
            .rpc_pool
            .get("retrieval")
            .map(|endpoint| endpoint.protocol.as_str()),
        Some("json-lines")
    );
    let profile = resources
        .agent_profiles
        .get("background.researcher")
        .unwrap();
    assert_eq!(profile.name.as_deref(), Some("Background Researcher"));
    assert_eq!(profile.role.as_deref(), Some("browser_operator"));
    assert_eq!(profile.features, vec!["delegated-reporting".to_string()]);
    assert_eq!(facade.config.rpc_tools.len(), 2);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn resource_registration_file_resolves_paths_from_resource_file_dir() {
    let root = unique_test_dir("resource-registration-file");
    let resources_dir = root.join("resources");
    let skills_dir = resources_dir.join("skills");
    let workflows_dir = resources_dir.join("workflows");
    let bin_dir = resources_dir.join("bin");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    fs::create_dir_all(&workflows_dir).unwrap();
    fs::create_dir_all(&bin_dir).unwrap();
    let mut workflow =
        corework::workflow::chain_compiler_v2::compile_chain_v2("input name\nreturn result=$name")
            .unwrap();
    workflow.metadata.id = "hello-workflow".to_string();
    workflow.metadata.name = "Hello Workflow".to_string();
    workflow.metadata.description = "Test workflow registry entry".to_string();
    workflow
        .save_to_workflow_file(workflows_dir.join("hello.workflow.json"))
        .unwrap();

    let resource_path = resources_dir.join("resources.json");
    fs::write(
        &resource_path,
        json!({
            "schema": "agent-runtime-resource-registration/v1",
            "id": "default-resources",
            "skills": {
                "root_dir": "./skills",
                "builtin_system": true
            },
            "workflows": {
                "root_dir": "./workflows",
                "registry_id": "default-workflows"
            },
            "data": {
                "data_dir": "./data/runtime",
                "logs_dir": "./data/runtime/logs"
            },
            "rpc_endpoints": [{
                "id": " browser-tools ",
                "protocol": " grpc ",
                "endpoint": " http://127.0.0.1:50051 ",
                "launch": {
                    "kind": "process",
                    "program": "./bin/browser-tools.exe",
                    "working_dir": "./bin"
                }
            }]
        })
        .to_string(),
    )
    .unwrap();

    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_file(resource_path.to_str().unwrap())
        .unwrap();

    let resources = facade.registered_resources().unwrap();
    assert_eq!(resources.skills_root_dir, skills_dir);
    assert_eq!(
        resources.workflows_root_dir.as_deref(),
        Some(workflows_dir.as_path())
    );
    assert_eq!(
        resources.workflow_registry_id.as_deref(),
        Some("default-workflows")
    );
    assert_eq!(resources.workflow_registry.len(), 1);
    assert_eq!(resources.workflow_registry[0].id, "hello-workflow");
    assert_eq!(resources.workflow_registry[0].name, "Hello Workflow");
    assert_eq!(
        resources.workflow_registry[0].file_name,
        "hello.workflow.json"
    );
    assert_eq!(
        resources.workflow_registry[0].file_path,
        workflows_dir.join("hello.workflow.json")
    );
    assert_eq!(
        resources.data_dir.as_deref(),
        Some(resources_dir.join("data/runtime").as_path())
    );
    assert_eq!(
        facade.config.workflow.auto_load_dir.as_deref(),
        Some(workflows_dir.as_path())
    );
    assert_eq!(resources.rpc_pool.len(), 1);
    let endpoint = resources.rpc_pool.get("browser-tools").unwrap();
    assert_eq!(endpoint.protocol, "grpc");
    assert_eq!(endpoint.address, "http://127.0.0.1:50051");
    let launch = endpoint.launch.as_ref().unwrap();
    assert_eq!(
        launch.program.as_deref(),
        Some(bin_dir.join("browser-tools.exe").as_path())
    );
    assert_eq!(launch.working_dir.as_deref(), Some(bin_dir.as_path()));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn llm_registration_builds_gateway_config_and_registry() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("llm-registration");
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();

    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "current_model_uid": 1001,
                "providers": [{
                    "id": 1,
                    "name": "deepseek-main",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "api_paradigm": "openai_chat_completions",
                    "enabled_models": [{
                        "uid": 1001,
                        "model_id": "deepseek-v4-flash",
                        "max_context_tokens": 1000000
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();

    let registry = facade.registered_llm().unwrap();
    assert_eq!(registry.id, "default-llm");
    assert_eq!(registry.current_model_uid, Some(1001));
    assert_eq!(registry.provider_count, 1);
    assert_eq!(registry.model_count, 1);
    assert_eq!(facade.llm_config.current_model_uid, Some(1001));
    assert_eq!(facade.llm_config.providers[0].id, 1);
    assert_eq!(facade.llm_config.providers[0].enabled_models[0].uid, 1001);
    assert!(key_store::get(1001).is_some());

    let persisted = fs::read_to_string(root.join("llm_config.json")).unwrap();
    let persisted: llm_gateway::LlmConfig = serde_json::from_str(&persisted).unwrap();
    assert_eq!(persisted.current_model_uid, Some(1001));
    assert_eq!(persisted.providers[0].api_key, "sk-test");

    let mut restarted = RuntimeFacade::create(&create_options).unwrap();
    restarted
        .register_llm_file(root.join("llm_config.json").to_str().unwrap())
        .unwrap();
    assert_eq!(restarted.registered_llm().unwrap().id, "default-llm");
    assert_eq!(restarted.llm_config.current_model_uid, Some(1001));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn llm_registration_allows_prompt_cache_for_anthropic_messages() {
    let registration: LlmRegistration = serde_json::from_value(json!({
        "schema": "agent-runtime-llm-registration/v1",
        "id": "anthropic-llm",
        "providers": [{
            "id": 1,
            "name": "claude-main",
            "type": "claude",
            "api_paradigm": "anthropic_messages",
            "prompt_cache_control": true,
            "enabled_models": [{
                "uid": 1001,
                "model_id": "claude-sonnet"
            }]
        }]
    }))
    .unwrap();

    let (_, config) = build_llm_registry_and_config(registration).unwrap();
    assert!(config.providers[0].prompt_cache_control);
}

#[test]
fn llm_registration_rejects_prompt_cache_for_non_anthropic_provider() {
    let registration: LlmRegistration = serde_json::from_value(json!({
        "schema": "agent-runtime-llm-registration/v1",
        "id": "openai-llm",
        "providers": [{
            "id": 1,
            "name": "openai-main",
            "type": "openai",
            "api_paradigm": "openai_chat_completions",
            "prompt_cache_control": true,
            "enabled_models": [{
                "uid": 1001,
                "model_id": "gpt-test"
            }]
        }]
    }))
    .unwrap();

    let error = build_llm_registry_and_config(registration).unwrap_err();
    assert!(error
        .to_string()
        .contains("prompt_cache_control requires api_paradigm 'anthropic_messages'"));
}

#[test]
fn llm_registration_rejects_removed_tools_field() {
    let error = serde_json::from_value::<LlmRegistration>(json!({
        "schema": "agent-runtime-llm-registration/v1",
        "id": "default-llm",
        "tools": {}
    }))
    .unwrap_err();

    assert!(error.to_string().contains("tools"));
}

#[test]
fn agent_cluster_registration_rejects_removed_tools_field() {
    let error = serde_json::from_value::<AgentClusterRegistration>(json!({
        "schema": "agent-runtime-agent-cluster-registration/v1",
        "id": "default-cluster",
        "description": "Test cluster",
        "agents": [{
            "id": "agent-a",
            "name": "Agent A"
        }],
        "tools": {
            "rpc": ["OpenPage"],
            "builtin": ["RagRetrieve"]
        }
    }))
    .unwrap_err();

    assert!(error.to_string().contains("tools"));
}

#[test]
fn spawn_conversation_request_rejects_host_owned_identity_fields() {
    let error = serde_json::from_value::<ConversationSpawnRequest>(json!({
        "schema": "agent-runtime-conversation-spawn/v1",
        "cluster_id": "default-cluster",
        "conversation_id": "host-provided",
        "tenant_id": "tenant",
        "user_id": "user"
    }))
    .unwrap_err();

    assert!(error.to_string().contains("conversation_id"));
}

#[test]
fn agent_cluster_registration_builds_registered_cluster() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("agent-cluster-registration");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                },
                "rpc_endpoints": [{
                    "id": "retrieval",
                    "protocol": "json-lines",
                    "endpoint": "127.0.0.1:47071"
                }]
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "current_model_uid": 1001,
                "providers": [{
                    "id": 1,
                    "name": "deepseek-main",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "api_paradigm": "openai_chat_completions",
                    "enabled_models": [{
                        "uid": 1001,
                        "model_id": "deepseek-v4-flash"
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "default-cluster",
                "description": "Test cluster",
                "focus_agent_id": "agent-a",
                "agents": [{
                    "id": "agent-a",
                    "name": "Agent A",
                    "role": "browser_operator",
                    "features": ["after_sales"],
                    "model_uid": 1001,
                    "retrieval": {
                        "enabled": true,
                        "endpoint_id": "retrieval",
                        "tool_name": "RagRetrieve"
                    },
                    "system_prompt_constraints": {
                        "frontend_widgets_enabled": false
                    }
                }],
                "permissions": {
                    "read_only": "full",
                    "controlled_change": "ask",
                    "destructive": "deny"
                },
                "max_thinking_rounds": 0
            })
            .to_string(),
        )
        .unwrap();

    let cluster = facade
        .registries
        .agent_clusters
        .get("default-cluster")
        .unwrap();
    assert_eq!(cluster.description, "Test cluster");
    assert_eq!(cluster.focus_agent_id, "agent-a");
    assert_eq!(cluster.agents[0].model_uid, 1001);
    assert!(!cluster.agents[0].frontend_widgets_enabled);
    assert_eq!(
        cluster.agents[0]
            .system_prompt_constraints
            .frontend_widgets_enabled,
        Some(false)
    );
    assert!(cluster.agents[0].retrieval.as_ref().unwrap().enabled);
    assert_eq!(
        cluster.permissions.read_only,
        ai_assistant::ToolPermissionMode::Full
    );
    assert_eq!(
        cluster.permissions.controlled_change,
        ai_assistant::ToolPermissionMode::Ask
    );
    assert_eq!(
        cluster.permissions.destructive,
        ai_assistant::ToolPermissionMode::Deny
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn agent_cluster_without_model_uses_deferred_model_configuration() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("agent-cluster-no-current-model");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                }
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "current_model_uid": null,
                "providers": [{
                    "id": 1,
                    "name": "deepseek-main",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "api_paradigm": "openai_chat_completions",
                    "enabled_models": [{
                        "uid": 1001,
                        "model_id": "deepseek-v4-flash"
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();

    facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "default-cluster",
                "description": "Test cluster",
                "focus_agent_id": "agent-a",
                "agents": [{
                    "id": "agent-a",
                    "name": "Agent A",
                    "role": "browser_operator"
                }]
            })
            .to_string(),
        )
        .unwrap();
    let cluster = facade.registered_cluster_description("default-cluster");
    assert_eq!(cluster, Some("Test cluster"));
    assert_eq!(
        facade
            .registries
            .agent_clusters
            .get("default-cluster")
            .unwrap()
            .agents[0]
            .model_uid,
        0
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn agent_cluster_registration_expands_agent_profiles_by_name() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("agent-cluster-profile-name");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                },
                "agents": {
                    "profiles": [{
                        "id": "background.researcher",
                        "name": "Background Researcher",
                        "role": "browser_operator",
                        "features": ["after_sales"],
                        "retrieval": {
                            "enabled": true,
                            "endpoint_id": "knowledge-general",
                            "profiles": ["general"]
                        }
                    }]
                },
                "rpc_endpoints": [{
                    "id": "knowledge-general",
                    "protocol": "json-lines",
                    "endpoint": "127.0.0.1:47071"
                }, {
                    "id": "knowledge-legal",
                    "protocol": "json-lines",
                    "endpoint": "127.0.0.1:47072"
                }]
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "current_model_uid": 1001,
                "providers": [{
                    "id": 1,
                    "name": "deepseek-main",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "enabled_models": [{
                        "uid": 1001,
                        "model_id": "deepseek-v4-flash"
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "default-cluster",
                "description": "Test cluster",
                "focus_agent_id": "researcher-a",
                "agents": [{
                    "id": "researcher-a",
                    "profile": "background.researcher",
                    "name": "Researcher A"
                }, {
                    "id": "researcher-b",
                    "profile": "background.researcher",
                    "name": "Researcher B",
                    "retrieval": {
                        "enabled": true,
                        "endpoint_id": "knowledge-legal",
                        "profiles": ["legal"]
                    }
                }]
            })
            .to_string(),
        )
        .unwrap();

    let cluster = facade
        .registries
        .agent_clusters
        .get("default-cluster")
        .unwrap();
    let agent = &cluster.agents[0];
    assert_eq!(cluster.focus_agent_id, "researcher-a");
    assert_eq!(agent.id, "researcher-a");
    assert_eq!(agent.name, "Researcher A");
    assert_eq!(agent.role.as_deref(), Some("browser_operator"));
    assert_eq!(agent.features, vec!["after_sales".to_string()]);
    assert_eq!(
        agent.retrieval.as_ref().unwrap().endpoint_id.as_deref(),
        Some("knowledge-general")
    );
    assert_eq!(
        cluster.agents[1]
            .retrieval
            .as_ref()
            .unwrap()
            .endpoint_id
            .as_deref(),
        Some("knowledge-legal")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn agent_cluster_focus_profile_id_must_be_unambiguous() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("agent-cluster-focus-profile-ambiguous");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                },
                "agents": {
                    "profiles": [{
                        "id": "background.researcher",
                        "name": "Background Researcher",
                        "role": "browser_operator"
                    }]
                }
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "current_model_uid": 1001,
                "providers": [{
                    "id": 1,
                    "name": "deepseek-main",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "enabled_models": [{
                        "uid": 1001,
                        "model_id": "deepseek-v4-flash"
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();

    let error = facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "default-cluster",
                "description": "Test cluster",
                "focus_agent_id": "background.researcher",
                "agents": [{
                    "id": "researcher-a",
                    "profile": "background.researcher",
                    "name": "Researcher A"
                }, {
                    "id": "researcher-b",
                    "profile": "background.researcher",
                    "name": "Researcher B"
                }]
            })
            .to_string(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("matches multiple agents; use a concrete agent id"),
        "{error}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn builtin_studio_clusters_are_separate_from_business_registry() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("builtin-cluster-isolation");
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "current_model_uid": 1001,
                "providers": [{
                    "id": 1,
                    "name": "test-provider",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "enabled_models": [{
                        "uid": 1001,
                        "model_id": "test-model"
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "business-cluster",
                "description": "Business cluster",
                "agents": [{
                    "id": "business-agent",
                    "name": "Business Agent",
                    "role": "business_role",
                    "model_uid": 1001
                }]
            })
            .to_string(),
        )
        .unwrap();

    let builtin = build_builtin_cluster_configs(&facade.registries).unwrap();

    assert_eq!(
        facade
            .registries
            .agent_clusters
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["business-cluster".to_string()]
    );
    assert_eq!(builtin.workflow_editor.id, "workflow-studio");
    assert_eq!(
        builtin.agent_test_supervisor.focus_agent_id,
        "agent-test-supervisor"
    );
    assert_eq!(
        builtin.agent_test_adversary.focus_agent_id,
        "agent-test-adversary"
    );
    assert_ne!(
        builtin.agent_test_supervisor.id,
        builtin.agent_test_adversary.id
    );
    assert_eq!(
        builtin.agent_test_supervisor.agents[0].role.as_deref(),
        Some("agent_test_supervisor")
    );
    assert_eq!(
        builtin.agent_test_adversary.agents[0].role.as_deref(),
        Some("agent_test_adversary")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn spawn_conversation_returns_generated_id_and_initializes_focus_agent_cache() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("spawn-conversation");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                },
                "data": {
                    "logs_dir": root.join("conversation-logs")
                }
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "current_model_uid": 1001,
                "providers": [{
                    "id": 1,
                    "name": "deepseek-main",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "api_paradigm": "openai_chat_completions",
                    "enabled_models": [{
                        "uid": 1001,
                        "model_id": "deepseek-v4-flash"
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "default-cluster",
                "description": "Test cluster",
                "focus_agent_id": "agent-a",
                "agents": [{
                    "id": "agent-a",
                    "name": "Agent A",
                    "role": "browser_operator",
                    "model_uid": 1001
                }, {
                    "id": "agent-b",
                    "name": "Agent B",
                    "role": "browser_operator",
                    "model_uid": 1001
                }],
                "max_thinking_rounds": 0
            })
            .to_string(),
        )
        .unwrap();
    facade.start().unwrap();

    let info = facade
        .spawn_conversation(
            &json!({
                "schema": "agent-runtime-conversation-spawn/v1",
                "cluster_id": "default-cluster"
            })
            .to_string(),
        )
        .unwrap();

    assert!(!info.conversation_id.is_empty());
    let manager = facade.manager().unwrap();
    facade.rt.block_on(async {
        let cache = manager
            .default_agent_cache(&info.conversation_id)
            .await
            .unwrap();
        let max_rounds = cache
            .get::<u32>(ai_assistant::context::keys::MAX_THINKING_ROUNDS)
            .await
            .unwrap();
        assert_eq!(max_rounds, Some(0));
    });

    let log_path = facade
        .conversation_instances
        .get(&info.conversation_id)
        .and_then(|metadata| metadata.log_path.clone())
        .expect("conversation log path");
    assert!(log_path.starts_with(root.join("conversation-logs").join("default-cluster")));
    facade.close_conversation(&info.conversation_id).unwrap();
    facade.rt.block_on(async {
        assert!(manager
            .conversation_status(&info.conversation_id)
            .await
            .is_err());
        let error = manager
            .send_message(&info.conversation_id, "must not be accepted after close")
            .await
            .expect_err("closed conversation must reject new messages");
        assert!(error.to_string().contains("not found"));
    });
    let log = fs::read_to_string(log_path).unwrap();
    assert!(log.contains("\"event\":\"conversation_created\""));
    assert!(log.contains("\"event\":\"conversation_closed\""));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn spawn_conversation_from_snapshot_allocates_a_new_runtime_id_and_restores_ledger() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("spawn-conversation-from-snapshot");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                }
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "current_model_uid": 1001,
                "providers": [{
                    "id": 1,
                    "name": "deepseek-main",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "api_paradigm": "openai_chat_completions",
                    "enabled_models": [{
                        "uid": 1001,
                        "model_id": "deepseek-v4-flash"
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "default-cluster",
                "description": "Test cluster",
                "focus_agent_id": "agent-a",
                "agents": [{
                    "id": "agent-a",
                    "name": "Agent A",
                    "role": "browser_operator",
                    "model_uid": 1001
                }],
                "max_thinking_rounds": 0
            })
            .to_string(),
        )
        .unwrap();
    facade.start().unwrap();

    let info = facade
        .spawn_conversation_from_snapshot(
            &json!({
                "schema": "agent-runtime-conversation-spawn/v1",
                "cluster_id": "default-cluster"
            })
            .to_string(),
            &json!({
                "schema": "agent-runtime-conversation-snapshot/v1",
                "conversation_id": "old-runtime-id",
                "ledger": [{
                    "record_id": 41,
                    "conversation_id": "old-runtime-id",
                    "agent_id": "agent-a",
                    "agent_name": "Agent A",
                    "role": "user",
                    "content": "restored message",
                    "metadata": {},
                    "created_at": "2026-06-12T18:00:00+08:00"
                }]
            })
            .to_string(),
        )
        .unwrap();

    assert_ne!(info.conversation_id, "old-runtime-id");
    let manager = facade.manager().unwrap();
    let records = facade
        .rt
        .block_on(manager.ledger(
            &info.conversation_id,
            ai_assistant::conversation_state::LedgerReadOptions::default(),
        ))
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].conversation_id, info.conversation_id);
    assert_eq!(records[0].record_id, 1);
    assert_eq!(records[0].content, "restored message");

    let repaired_info = facade
        .spawn_conversation_from_snapshot(
            &json!({
                "schema": "agent-runtime-conversation-spawn/v1",
                "cluster_id": "default-cluster"
            })
            .to_string(),
            &json!({
                "schema": "agent-runtime-conversation-snapshot/v1",
                "conversation_id": "old-runtime-id-with-open-tool",
                "ledger": [{
                    "record_id": 2,
                    "conversation_id": "old-runtime-id-with-open-tool",
                    "agent_id": "agent-a",
                    "agent_name": "Agent A",
                    "role": "gateway_message",
                    "content": "running WriteScript",
                    "metadata": {
                        "subtype": "tool_call_started",
                        "tool_name": "WriteScript",
                        "tool_command": "WriteScript --path a.md",
                        "extra": {
                            "call_id": "call-from-snapshot",
                            "status": "running"
                        }
                    },
                    "created_at": "2026-06-12T18:00:00+08:00"
                }]
            })
            .to_string(),
        )
        .unwrap();
    let repaired_records = facade
        .rt
        .block_on(manager.ledger(
            &repaired_info.conversation_id,
            ai_assistant::conversation_state::LedgerReadOptions::default(),
        ))
        .unwrap();
    assert_eq!(repaired_records.len(), 2);
    assert_eq!(repaired_records[0].record_id, 1);
    assert_eq!(repaired_records[1].record_id, 2);
    assert_eq!(
        repaired_records[1].metadata.extra.get("call_id"),
        Some(&json!("call-from-snapshot"))
    );
    assert_eq!(
        repaired_records[1].metadata.extra.get("status"),
        Some(&json!("recovery_interrupted"))
    );

    facade.close_conversation(&info.conversation_id).unwrap();
    facade
        .close_conversation(&repaired_info.conversation_id)
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn spawn_conversation_from_persisted_json_replays_state_deltas_and_repairs_open_tools() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("spawn-conversation-from-persisted-json");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                }
            })
            .to_string(),
        )
        .unwrap();
    register_test_llm(&mut facade, 1001);
    facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "default-cluster",
                "description": "Test cluster",
                "focus_agent_id": "agent-a",
                "agents": [{
                    "id": "agent-a",
                    "name": "Agent A",
                    "role": "browser_operator",
                    "model_uid": 1001
                }],
                "max_thinking_rounds": 0
            })
            .to_string(),
        )
        .unwrap();
    facade.start().unwrap();

    let info = facade
        .spawn_conversation_from_snapshot(
            &json!({
                "schema": "agent-runtime-conversation-spawn/v1",
                "cluster_id": "default-cluster"
            })
            .to_string(),
            &json!({
                "schema": "agent-runtime-conversation-snapshot/v1",
                "conversation_id": "old-runtime-id",
                "ledger": [{
                    "record_id": 7,
                    "conversation_id": "old-runtime-id",
                    "agent_id": "agent-a",
                    "agent_name": "Agent A",
                    "role": "gateway_message",
                    "content": "running UpdateOrder",
                    "metadata": {
                        "subtype": "tool_call_started",
                        "tool_name": "UpdateOrder",
                        "tool_command": "UpdateOrder --id 42",
                        "extra": {
                            "call_id": "call-open-after-pod-crash",
                            "status": "running",
                            "effect": "write"
                        }
                    },
                    "created_at": "2026-06-23T10:00:00+08:00"
                }],
                "state_deltas": [{
                    "schema": "agent-runtime-state-delta/v1",
                    "op": "focus.set",
                    "conversation_id": "old-runtime-id",
                    "focus_agent_id": "agent-a"
                }, {
                    "schema": "agent-runtime-state-delta/v1",
                    "op": "dynamic_snapshot.set",
                    "conversation_id": "old-runtime-id",
                    "agent_id": "agent-a",
                    "field": "host:order_page",
                    "text": "Order page showed status=processing",
                    "host_owned": true,
                    "stale_after_restore": true
                }, {
                    "schema": "agent-runtime-state-delta/v1",
                    "op": "agent_task.upsert",
                    "conversation_id": "old-runtime-id",
                    "task": {
                        "task_id": "task-restore-1",
                        "title": "Verify order mutation",
                        "objective": "Check whether the interrupted write finished",
                        "acceptance": ["order status is known"],
                        "delegator_agent_id": "agent-a",
                        "delegator_agent_name": "Agent A",
                        "assignee_agent_id": "agent-a",
                        "assignee_agent_name": "Agent A",
                        "status": "running",
                        "created_at": "2026-06-23T10:00:01+08:00",
                        "updated_at": "2026-06-23T10:00:02+08:00"
                    }
                }, {
                    "schema": "agent-runtime-state-delta/v1",
                    "op": "agent_skills.set",
                    "conversation_id": "old-runtime-id",
                    "agent_id": "agent-a",
                    "agent_name": "Agent A",
                    "main_skills": ["browser_operator"],
                    "imported_skills": ["browser_operator"],
                    "active_tools": ["QueryLedgerSystem", "WriteScript"]
                }, {
                    "schema": "agent-runtime-state-delta/v1",
                    "op": "agent_plan.set",
                    "conversation_id": "old-runtime-id",
                    "agent_id": "agent-a",
                    "agent_name": "Agent A",
                    "plan": {
                        "title": "Recover interrupted order work",
                        "summary": "Need to verify interrupted write before continuing.",
                        "content": "- Check order 42\n- Tell user if the write is uncertain",
                        "status": "active",
                        "created_at": "2026-06-23T10:00:03+08:00",
                        "updated_at": "2026-06-23T10:00:04+08:00"
                    }
                }, {
                    "schema": "agent-runtime-state-delta/v1",
                    "op": "agent_task.upsert",
                    "task": { "task_id": 123 }
                }]
            })
            .to_string(),
        )
        .unwrap();

    let manager = facade.manager().unwrap();
    facade.rt.block_on(async {
        let records = manager
            .ledger(
                &info.conversation_id,
                ai_assistant::conversation_state::LedgerReadOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].conversation_id, info.conversation_id);
        assert_eq!(records[0].record_id, 1);
        assert_eq!(records[1].record_id, 2);
        assert_eq!(
            records[1].metadata.extra.get("call_id"),
            Some(&json!("call-open-after-pod-crash"))
        );
        assert_eq!(
            records[1].metadata.extra.get("status"),
            Some(&json!("recovery_interrupted"))
        );
        assert_eq!(
            records[1].metadata.extra.get("effect"),
            Some(&json!("unknown"))
        );
        assert!(records[1]
            .content
            .contains("tool_call_id: call-open-after-pod-crash"));

        let status = manager
            .conversation_status(&info.conversation_id)
            .await
            .unwrap();
        assert_eq!(status.active_agent_id, "agent-a");

        let snapshots = manager
            .host_dynamic_snapshots(&info.conversation_id, "agent-a")
            .await
            .unwrap();
        assert_eq!(
            snapshots.get("host:order_page").map(String::as_str),
            Some("Order page showed status=processing")
        );

        let tasks = manager.agent_tasks(&info.conversation_id).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_id, "task-restore-1");
        assert_eq!(
            tasks[0].status,
            ai_assistant::conversation_state::AgentTaskStatus::Running
        );

        let cache = manager
            .agent_cache(&info.conversation_id, "agent-a")
            .await
            .unwrap();
        let main_skills = cache
            .get::<Vec<String>>(ai_assistant::context::keys::MAIN_SKILLS)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(main_skills, vec!["browser_operator".to_string()]);
        let imported_skills = cache
            .get::<Vec<String>>(ai_assistant::context::keys::IMPORTED_SKILLS)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(imported_skills, vec!["browser_operator".to_string()]);
        let active_tools = cache
            .get::<Vec<String>>(ai_assistant::context::keys::ACTIVE_TOOLS)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            active_tools,
            vec!["QueryLedgerSystem".to_string(), "WriteScript".to_string()]
        );
        let plan = AssistantContext::get_current_plan(&cache)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(plan.title, "Recover interrupted order work");
        assert!(plan.is_active());
    });

    facade.close_conversation(&info.conversation_id).unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn conversation_snapshot_import_accepts_state_delta_shapes() {
    let snapshot = parse_conversation_snapshot(
        &json!({
            "schema": "agent-runtime-conversation-snapshot/v1",
            "conversation_id": "old-runtime-id",
            "ledger": [],
            "state_delta": {
                "schema": "agent-runtime-state-delta/v1",
                "op": "focus.set",
                "focus_agent_id": "agent-a"
            },
            "state": {
                "deltas": [{
                    "schema": "agent-runtime-state-delta/v1",
                    "op": "dynamic_snapshot.set",
                    "agent_id": "agent-a",
                    "field": "host:page",
                    "text": "snapshot"
                }],
                "state_deltas": [{
                    "schema": "agent-runtime-state-delta/v1",
                    "op": "agent_task.upsert",
                    "task": {
                        "task_id": "task-1",
                        "title": "Restore task",
                        "objective": "Restore task board",
                        "delegator_agent_id": "agent-a",
                        "delegator_agent_name": "Agent A",
                        "status": "pending",
                        "created_at": "2026-06-23T10:00:00+08:00",
                        "updated_at": "2026-06-23T10:00:00+08:00"
                    }
                }]
            }
        })
        .to_string(),
    )
    .unwrap();

    let deltas = snapshot_state_deltas(&snapshot);
    assert_eq!(deltas.len(), 3);
    assert_eq!(deltas[0]["op"], "focus.set");
    assert_eq!(deltas[1]["op"], "dynamic_snapshot.set");
    assert_eq!(deltas[2]["op"], "agent_task.upsert");
}

#[test]
fn restored_ledger_builds_execution_plan_without_appending_tool_result() {
    let records = vec![ai_assistant::ledger::LedgerRecord {
        record_id: 1,
        conversation_id: "old-runtime-id".to_string(),
        agent_id: "agent-a".to_string(),
        agent_name: "Agent A".to_string(),
        role: ai_assistant::ledger::LedgerRole::GatewayMessage,
        content: "running UpdateOrder".to_string(),
        metadata: ai_assistant::ledger::LedgerMessageMeta {
            subtype: Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_STARTED.to_string()),
            tool_name: Some("UpdateOrder".to_string()),
            tool_command: Some("UpdateOrder --id 42".to_string()),
            extra: BTreeMap::from([
                ("call_id".to_string(), json!("call-open")),
                ("status".to_string(), json!("running")),
            ]),
            ..Default::default()
        },
        created_at: "2026-06-23T10:00:00+08:00".to_string(),
    }];

    let plans = restored_execution_plans(&records);

    assert_eq!(records.len(), 1);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].agent_id, "agent-a");
    assert_eq!(plans[0].tools, vec!["UpdateOrder --id 42".to_string()]);
    assert_eq!(plans[0].call_ids, vec!["call-open".to_string()]);
    let recovery = plans[0]
        .recovery_results
        .get("call-open")
        .expect("non-readonly tool should be closed by recovery result");
    assert_eq!(recovery.result["status"], "recovery_interrupted");
    assert!(recovery.to_ai.contains("tool_call_id: call-open"));
}

#[test]
fn restored_ledger_readonly_open_tool_has_no_recovery_result() {
    ai_assistant::runtime_tools::register_runtime_tool(corework::rpc_tool::RuntimeToolMetadata {
        name: "ReadonlyRecoveryProbe".to_string(),
        display_name: "Readonly Recovery Probe".to_string(),
        description: String::new(),
        tool_kind: "rpc".to_string(),
        parameters: vec![],
        outputs: vec![],
        destructive: false,
        readonly: true,
        idempotent: true,
        open_world: false,
        secret: false,
        required_capabilities: vec![],
        endpoint_id: "test".to_string(),
        service: "test".to_string(),
        method: "execute".to_string(),
    });
    let records = vec![ai_assistant::ledger::LedgerRecord {
        record_id: 1,
        conversation_id: "old-runtime-id".to_string(),
        agent_id: "agent-a".to_string(),
        agent_name: "Agent A".to_string(),
        role: ai_assistant::ledger::LedgerRole::GatewayMessage,
        content: "running ReadonlyRecoveryProbe".to_string(),
        metadata: ai_assistant::ledger::LedgerMessageMeta {
            subtype: Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_STARTED.to_string()),
            tool_name: Some("ReadonlyRecoveryProbe".to_string()),
            tool_command: Some("ReadonlyRecoveryProbe".to_string()),
            extra: BTreeMap::from([
                ("call_id".to_string(), json!("call-readonly")),
                ("status".to_string(), json!("running")),
            ]),
            ..Default::default()
        },
        created_at: "2026-06-23T10:00:00+08:00".to_string(),
    }];

    let plans = restored_execution_plans(&records);

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tools, vec!["ReadonlyRecoveryProbe".to_string()]);
    assert_eq!(plans[0].call_ids, vec!["call-readonly".to_string()]);
    assert!(plans[0].recovery_results.is_empty());
}

#[test]
fn restored_ledger_groups_open_tools_by_agent() {
    let records = vec![
        ai_assistant::ledger::LedgerRecord {
            record_id: 1,
            conversation_id: "old-runtime-id".to_string(),
            agent_id: "agent-a".to_string(),
            agent_name: "Agent A".to_string(),
            role: ai_assistant::ledger::LedgerRole::GatewayMessage,
            content: "running UpdateOrder".to_string(),
            metadata: ai_assistant::ledger::LedgerMessageMeta {
                subtype: Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_STARTED.to_string()),
                tool_name: Some("UpdateOrder".to_string()),
                tool_command: Some("UpdateOrder --id 1".to_string()),
                extra: BTreeMap::from([
                    ("call_id".to_string(), json!("call-a")),
                    ("status".to_string(), json!("running")),
                ]),
                ..Default::default()
            },
            created_at: "2026-06-23T10:00:00+08:00".to_string(),
        },
        ai_assistant::ledger::LedgerRecord {
            record_id: 2,
            conversation_id: "old-runtime-id".to_string(),
            agent_id: "agent-b".to_string(),
            agent_name: "Agent B".to_string(),
            role: ai_assistant::ledger::LedgerRole::GatewayMessage,
            content: "running WriteScript".to_string(),
            metadata: ai_assistant::ledger::LedgerMessageMeta {
                subtype: Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_STARTED.to_string()),
                tool_name: Some("WriteScript".to_string()),
                tool_command: Some("WriteScript --path b.md".to_string()),
                extra: BTreeMap::from([
                    ("call_id".to_string(), json!("call-b")),
                    ("status".to_string(), json!("running")),
                ]),
                ..Default::default()
            },
            created_at: "2026-06-23T10:00:01+08:00".to_string(),
        },
    ];

    let plans = restored_execution_plans(&records);

    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0].agent_id, "agent-a");
    assert_eq!(plans[0].tools, vec!["UpdateOrder --id 1".to_string()]);
    assert_eq!(plans[0].call_ids, vec!["call-a".to_string()]);
    assert!(plans[0].recovery_results.contains_key("call-a"));
    assert_eq!(plans[1].agent_id, "agent-b");
    assert_eq!(plans[1].tools, vec!["WriteScript --path b.md".to_string()]);
    assert_eq!(plans[1].call_ids, vec!["call-b".to_string()]);
    assert!(plans[1].recovery_results.contains_key("call-b"));
}

#[test]
fn restored_ledger_tail_user_enters_thinking() {
    let records = vec![ai_assistant::ledger::LedgerRecord {
        record_id: 1,
        conversation_id: "old-runtime-id".to_string(),
        agent_id: "agent-a".to_string(),
        agent_name: "Agent A".to_string(),
        role: ai_assistant::ledger::LedgerRole::User,
        content: "continue".to_string(),
        metadata: ai_assistant::ledger::LedgerMessageMeta::default(),
        created_at: "2026-06-23T10:00:00+08:00".to_string(),
    }];

    let recovery = restored_conversation_recovery(&records);

    assert!(recovery.execution_plans.is_empty());
    assert_eq!(
        recovery.entry_states,
        vec![(
            "agent-a".to_string(),
            ai_assistant::state::states::THINKING.to_string()
        )]
    );
}

#[test]
fn restored_ledger_tail_clean_assistant_enters_suspended() {
    let records = vec![ai_assistant::ledger::LedgerRecord {
        record_id: 1,
        conversation_id: "old-runtime-id".to_string(),
        agent_id: "agent-a".to_string(),
        agent_name: "Agent A".to_string(),
        role: ai_assistant::ledger::LedgerRole::Assistant,
        content: "Done.".to_string(),
        metadata: ai_assistant::ledger::LedgerMessageMeta::default(),
        created_at: "2026-06-23T10:00:00+08:00".to_string(),
    }];

    let recovery = restored_conversation_recovery(&records);

    assert!(recovery.execution_plans.is_empty());
    assert_eq!(
        recovery.entry_states,
        vec![(
            "agent-a".to_string(),
            ai_assistant::state::states::SUSPENDED.to_string()
        )]
    );
}

#[test]
fn restored_ledger_tail_closed_tool_enters_thinking() {
    let records = vec![ai_assistant::ledger::LedgerRecord {
        record_id: 1,
        conversation_id: "old-runtime-id".to_string(),
        agent_id: "agent-a".to_string(),
        agent_name: "Agent A".to_string(),
        role: ai_assistant::ledger::LedgerRole::GatewayMessage,
        content: "ok".to_string(),
        metadata: ai_assistant::ledger::LedgerMessageMeta {
            subtype: Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_FINISHED.to_string()),
            tool_name: Some("ReadLedger".to_string()),
            tool_command: Some("ReadLedger --limit 1".to_string()),
            success: Some(true),
            extra: BTreeMap::from([
                ("call_id".to_string(), json!("call-closed")),
                ("status".to_string(), json!("success")),
            ]),
            ..Default::default()
        },
        created_at: "2026-06-23T10:00:00+08:00".to_string(),
    }];

    let recovery = restored_conversation_recovery(&records);

    assert!(recovery.execution_plans.is_empty());
    assert_eq!(
        recovery.entry_states,
        vec![(
            "agent-a".to_string(),
            ai_assistant::state::states::THINKING.to_string()
        )]
    );
}

#[test]
fn restored_ledger_tail_assistant_tool_call_enters_executing() {
    let mut metadata = ai_assistant::ledger::LedgerMessageMeta::default();
    metadata
        .extra
        .insert("tool_call_ids".to_string(), json!(["call-2"]));
    let records = vec![ai_assistant::ledger::LedgerRecord {
        record_id: 1,
        conversation_id: "old-runtime-id".to_string(),
        agent_id: "agent-a".to_string(),
        agent_name: "Agent A".to_string(),
        role: ai_assistant::ledger::LedgerRole::Assistant,
        content: "EXEC QueryLedger --conversation_id current".to_string(),
        metadata,
        created_at: "2026-06-23T10:00:00+08:00".to_string(),
    }];

    let recovery = restored_conversation_recovery(&records);

    assert!(recovery.entry_states.is_empty());
    assert_eq!(recovery.execution_plans.len(), 1);
    assert_eq!(
        recovery.execution_plans[0].call_ids,
        vec!["call-2".to_string()]
    );
    assert_eq!(
        recovery.execution_plans[0].tools,
        vec!["QueryLedger --conversation_id current".to_string()]
    );
}

#[test]
fn restored_ledger_repairs_unfinished_started_tool_call() {
    let mut records = vec![ai_assistant::ledger::LedgerRecord {
        record_id: 99,
        conversation_id: "old-runtime-id".to_string(),
        agent_id: "agent-a".to_string(),
        agent_name: "Agent A".to_string(),
        role: ai_assistant::ledger::LedgerRole::GatewayMessage,
        content: "running WriteScript".to_string(),
        metadata: ai_assistant::ledger::LedgerMessageMeta {
            subtype: Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_STARTED.to_string()),
            tool_name: Some("WriteScript".to_string()),
            tool_command: Some("WriteScript --path a.md".to_string()),
            extra: BTreeMap::from([
                ("call_id".to_string(), json!("call-1")),
                ("status".to_string(), json!("running")),
                ("turn_id".to_string(), json!(7)),
            ]),
            ..Default::default()
        },
        created_at: "2026-06-12T18:00:00+08:00".to_string(),
    }];

    repair_restored_ledger(&mut records, "new-runtime-id");

    assert_eq!(records.len(), 2);
    let recovery = &records[1];
    assert_eq!(recovery.record_id, 100);
    assert_eq!(recovery.conversation_id, "new-runtime-id");
    assert_eq!(recovery.role, ai_assistant::ledger::LedgerRole::Tool);
    assert_eq!(
        recovery.metadata.subtype.as_deref(),
        Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_FAILED)
    );
    assert_eq!(
        recovery.metadata.extra.get("call_id"),
        Some(&json!("call-1"))
    );
    assert_eq!(
        recovery.metadata.extra.get("status"),
        Some(&json!("recovery_interrupted"))
    );
    assert!(recovery.content.contains("tool_call_id: call-1"));
    assert!(recovery
        .content
        .contains("do not directly repeat the same operation"));
}

#[test]
fn restored_ledger_repairs_assistant_declared_tool_call_without_start_fact() {
    let mut metadata = ai_assistant::ledger::LedgerMessageMeta::default();
    metadata
        .extra
        .insert("tool_call_ids".to_string(), json!(["call-2"]));
    let mut records = vec![ai_assistant::ledger::LedgerRecord {
        record_id: 1,
        conversation_id: "old-runtime-id".to_string(),
        agent_id: "agent-a".to_string(),
        agent_name: "Agent A".to_string(),
        role: ai_assistant::ledger::LedgerRole::Assistant,
        content: "EXEC QueryLedger --conversation_id current".to_string(),
        metadata,
        created_at: "2026-06-12T18:00:00+08:00".to_string(),
    }];

    repair_restored_ledger(&mut records, "new-runtime-id");

    assert_eq!(records.len(), 2);
    let recovery = &records[1];
    assert_eq!(recovery.role, ai_assistant::ledger::LedgerRole::Tool);
    assert_eq!(recovery.metadata.tool_name.as_deref(), Some("QueryLedger"));
    assert_eq!(
        recovery.metadata.extra.get("call_id"),
        Some(&json!("call-2"))
    );
    assert_eq!(
        recovery.metadata.extra.get("source"),
        Some(&json!("assistant_declared"))
    );
    assert!(recovery.content.contains("effect: unknown"));
    assert!(recovery
        .content
        .contains("do not directly repeat the same operation"));
}

#[test]
fn best_effort_conversation_snapshot_exports_ledger_when_not_waiting() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("best-effort-conversation-snapshot");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                }
            })
            .to_string(),
        )
        .unwrap();
    register_test_llm(&mut facade, 1001);
    facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "default-cluster",
                "description": "Test cluster",
                "focus_agent_id": "agent-a",
                "agents": [{
                    "id": "agent-a",
                    "name": "Agent A",
                    "role": "browser_operator",
                    "model_uid": 1001
                }],
                "max_thinking_rounds": 0
            })
            .to_string(),
        )
        .unwrap();
    facade.start().unwrap();

    let info = facade
        .spawn_conversation_from_snapshot(
            &json!({
                "schema": "agent-runtime-conversation-spawn/v1",
                "cluster_id": "default-cluster"
            })
            .to_string(),
            &json!({
                "schema": "agent-runtime-conversation-snapshot/v1",
                "conversation_id": "old-runtime-id",
                "ledger": [{
                    "record_id": 1,
                    "conversation_id": "old-runtime-id",
                    "agent_id": "agent-a",
                    "agent_name": "Agent A",
                    "role": "user",
                    "content": "save this even while stopping",
                    "metadata": {},
                    "created_at": "2026-06-12T18:00:00+08:00"
                }]
            })
            .to_string(),
        )
        .unwrap();

    let manager = facade.manager().unwrap();
    facade.rt.block_on(async {
        let cache = manager
            .default_agent_cache(&info.conversation_id)
            .await
            .unwrap();
        cache
            .set(ai_assistant::context::keys::PAUSE_REQUESTED, &true, None)
            .await
            .unwrap();
    });

    let stable_error = facade
        .export_conversation_snapshot(&info.conversation_id, "{}")
        .unwrap_err();
    assert!(stable_error
        .to_string()
        .contains("conversation_not_waiting"));

    let snapshot: Value = serde_json::from_str(
        &facade
            .export_conversation_snapshot(
                &info.conversation_id,
                &json!({ "consistency": "best_effort" }).to_string(),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(snapshot["schema"], "agent-runtime-conversation-snapshot/v1");
    assert_eq!(snapshot["consistency"], "best_effort");
    assert_eq!(snapshot["stable"], false);
    assert_eq!(snapshot["conversation_state"], "stopping");
    assert_eq!(
        snapshot["ledger"][0]["content"],
        "save this even while stopping"
    );
    assert_eq!(snapshot["runtime"]["agents"][0]["agent_id"], "agent-a");

    facade.close_conversation(&info.conversation_id).unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn spawn_conversation_records_lifecycle_event_for_sse() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("spawn-conversation-event");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                }
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "current_model_uid": 1001,
                "providers": [{
                    "id": 1,
                    "name": "deepseek-main",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "enabled_models": [{
                        "uid": 1001,
                        "model_id": "deepseek-v4-flash"
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "default-cluster",
                "description": "Test cluster",
                "agents": [{
                    "id": "agent-a",
                    "name": "Agent A",
                    "role": "browser_operator",
                    "model_uid": 1001
                }]
            })
            .to_string(),
        )
        .unwrap();
    facade.start().unwrap();

    let info = facade
        .spawn_conversation(
            &json!({
                "schema": "agent-runtime-conversation-spawn/v1",
                "cluster_id": "default-cluster"
            })
            .to_string(),
        )
        .unwrap();

    let event = facade
        .event_log
        .lock()
        .unwrap()
        .iter()
        .find(|event| event["type"] == "conversation:created")
        .cloned()
        .expect("conversation:created event");
    assert_eq!(event["source"], CONVERSATION_CREATED_EVENT);
    assert_eq!(event["conversation_id"], info.conversation_id);
    assert_eq!(event["payload"]["conversation_id"], info.conversation_id);
    assert_eq!(event["payload"]["cluster_id"], "default-cluster");
    assert_eq!(event["payload"]["cluster_description"], "Test cluster");

    facade
        .rt
        .block_on(facade.event_bus.publish(BaseEvent::new(
            ai_assistant::events::types::CONVERSATION_LEDGER_DELTA,
            json!({
                "schema": "agent-runtime-ledger-delta/v1",
                "op": "append",
                "record_id": 1,
                "conversation_id": info.conversation_id,
                "record": {
                    "record_id": 1,
                    "conversation_id": info.conversation_id,
                    "agent_id": "agent-a",
                    "agent_name": "Agent A",
                    "role": "user",
                    "content": "replicate through runtime event stream",
                    "metadata": {},
                    "created_at": "2026-06-23T10:00:00+08:00"
                }
            }),
        )))
        .unwrap();
    let ledger_delta = facade
        .event_log
        .lock()
        .unwrap()
        .iter()
        .find(|event| event["type"] == "conversation.ledger_delta")
        .cloned()
        .expect("conversation.ledger_delta event");
    assert_eq!(ledger_delta["conversation_id"], info.conversation_id);
    assert_eq!(ledger_delta["payload"]["op"], "append");
    assert_eq!(ledger_delta["payload"]["record_id"], 1);
    assert_eq!(
        ledger_delta["payload"]["record"]["content"],
        "replicate through runtime event stream"
    );

    facade.close_conversation(&info.conversation_id).unwrap();
    let closed = facade
        .event_log
        .lock()
        .unwrap()
        .iter()
        .find(|event| event["type"] == "conversation:closed")
        .cloned()
        .expect("conversation:closed event");
    assert_eq!(closed["source"], CONVERSATION_CLOSED_EVENT);
    assert_eq!(closed["payload"]["conversation_id"], info.conversation_id);
    assert_eq!(closed["payload"]["cluster_id"], "default-cluster");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn spawn_conversation_rejects_removed_spawn_cache_field() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("spawn-cache-unknown-field");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                }
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "current_model_uid": 1001,
                "providers": [{
                    "id": 1,
                    "name": "deepseek-main",
                    "type": "deepseek",
                    "base_url": "https://api.deepseek.com",
                    "api_key": "sk-test",
                    "enabled_models": [{
                        "uid": 1001,
                        "model_id": "deepseek-v4-flash"
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();
    facade
        .register_agent_cluster_json(
            &json!({
                "schema": "agent-runtime-agent-cluster-registration/v1",
                "id": "default-cluster",
                "description": "Test cluster",
                "agents": [{
                    "id": "agent-a",
                    "name": "Agent A",
                    "role": "browser_operator",
                    "model_uid": 1001
                }]
            })
            .to_string(),
        )
        .unwrap();

    let mut spawn_request = json!({
        "schema": "agent-runtime-conversation-spawn/v1",
        "cluster_id": "default-cluster"
    });
    spawn_request
        .as_object_mut()
        .unwrap()
        .insert(["immutable", "cache"].join("_"), json!({ "agent-a": {} }));

    let error = facade
        .spawn_conversation(&spawn_request.to_string())
        .unwrap_err();

    assert!(error.to_string().contains("unknown field"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn llm_registration_is_frozen_after_runtime_start() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("llm-registration-frozen");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                }
            })
            .to_string(),
        )
        .unwrap();
    register_test_llm(&mut facade, 1001);
    facade.start().unwrap();

    let error = facade
        .register_llm_json(
            &json!({
                "schema": "agent-runtime-llm-registration/v1",
                "id": "default-llm",
                "providers": []
            })
            .to_string(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("llm registration is frozen after runtime start"),
        "{error}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn llm_config_can_reload_after_runtime_start() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("llm-reload-after-start");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "browser_operator",
        "role",
        "# Persona\n\nOperate browser.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                }
            })
            .to_string(),
        )
        .unwrap();
    register_test_llm(&mut facade, 1001);
    facade.start().unwrap();

    let llm_path = root.join("llm-providers.json");
    fs::write(
        &llm_path,
        json!({
            "schema": "agent-runtime-llm-registration/v1",
            "id": "reloaded-llm",
            "providers": [{
                "id": 77,
                "name": "Reloaded Provider",
                "type": "openai",
                "base_url": "https://reload.example/v1",
                "api_key": "sk-reload",
                "api_paradigm": "openai_chat_completions",
                "enabled_models": [{
                    "uid": 7701,
                    "model_id": "reload-chat"
                }]
            }],
            "current_model_uid": 7701
        })
        .to_string(),
    )
    .unwrap();

    facade.reload_llm_input(llm_path.to_str().unwrap()).unwrap();

    assert_eq!(key_store::current(), Some(7701));
    let definitions: Value = serde_json::from_str(&facade.provider_definitions().unwrap()).unwrap();
    assert_eq!(definitions["current_model_uid"], json!(7701));
    assert_eq!(definitions["models"][0]["model_name"], json!("reload-chat"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn rpc_tool_endpoint_rejects_removed_snapshot_boundary_field() {
    let error = serde_json::from_value::<RpcToolEndpointConfig>(json!({
        "endpoint_id": "legacy-tool",
        "address": "127.0.0.1:50051",
        "allowed_snapshot_prefixes": ["legacy:"]
    }))
    .unwrap_err();

    assert!(error.to_string().contains("allowed_snapshot_prefixes"));
}

#[test]
fn runtime_config_rejects_top_level_retrieval() {
    let mut config = RuntimeConfig::default();
    config.runtime.skills_dir = Some(PathBuf::from("skills"));
    config.retrieval = Some(RetrievalConfig::default());

    let err = validate_runtime_config(&config).unwrap_err();
    let message = err.to_string();

    assert!(
        message.contains("top-level retrieval is no longer supported"),
        "{message}"
    );
}

#[test]
fn runtime_config_accepts_agent_level_retrieval() {
    let mut config = RuntimeConfig::default();
    config.runtime.skills_dir = Some(PathBuf::from("skills"));
    config.agents[0].retrieval = Some(RetrievalConfig {
        enabled: true,
        endpoint_id: Some("knowledge-a".to_string()),
        ..RetrievalConfig::default()
    });
    config.rpc_tools.push(RpcToolEndpointConfig {
        endpoint_id: "knowledge-a".to_string(),
        address: "127.0.0.1:47071".to_string(),
        protocol: "json-lines".to_string(),
        ..RpcToolEndpointConfig::default()
    });

    validate_runtime_config(&config).unwrap();
}

#[test]
fn runtime_create_options_accept_direct_parameters() {
    let root = unique_test_dir("runtime-create-options");
    let config = parse_config_input(
        &json!({
            "schema": "agent-runtime-create-options/v1",
            "log_level": "debug",
            "language": "zh-CN",
            "restore_policy": "strict",
            "data_dir": root.join("data")
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(config.runtime.log_level, "debug");
    assert_eq!(config.runtime.language, "zh-CN");
    assert_eq!(config.runtime.data_dir, Some(root.join("data")));
    assert!(config.runtime.skills_dir.is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn runtime_create_rejects_config_file_paths() {
    let error = parse_config_input("config/runtime-config.json").unwrap_err();
    assert!(error
        .to_string()
        .contains("config file paths are not supported"));
}

#[test]
fn runtime_create_rejects_old_runtime_config_schema() {
    let error =
        parse_config_input(r#"{"schema":"agent-runtime-config/v1","runtime":{}}"#).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("unknown field") && message.contains("runtime"));
}

#[tokio::test]
async fn start_validation_requires_loadable_default_role_skill() {
    let root = unique_test_dir("role-skill-validation");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "order_admin",
        "role",
        "# Persona\n\nHandle orders.",
    );
    let mut config = RuntimeConfig::default();
    config.runtime.skills_dir = Some(skills_dir);
    config.agents = vec![AgentSection {
        id: "agent-a".to_string(),
        name: "Agent A".to_string(),
        is_default: true,
        role: Some("order_admin".to_string()),
        ..AgentSection::default()
    }];

    validate_default_agent_role_skill(&config).await.unwrap();
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn start_validation_rejects_missing_default_role_skill() {
    let root = unique_test_dir("missing-role-skill");
    let mut config = RuntimeConfig::default();
    config.runtime.skills_dir = Some(root.join("skills"));
    config.agents = vec![AgentSection {
        id: "agent-a".to_string(),
        name: "Agent A".to_string(),
        is_default: true,
        role: Some("order_admin".to_string()),
        ..AgentSection::default()
    }];

    let err = validate_default_agent_role_skill(&config)
        .await
        .unwrap_err();

    assert!(err
        .to_string()
        .contains("role skill 'order_admin' failed to load"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn runtime_facade_dynamic_snapshot_reaches_agent_prompt_context() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("dynamic-snapshot-runtime");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "sunwoo_support",
        "role",
        "# Persona\n\nHelp with Sunwoo audio conversion.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade.config.runtime.cluster_id = "sunwoo-ai-support".to_string();
    facade.config.runtime.runtime_instance_id = "sunwoo-desktop-local-runtime-test".to_string();
    facade.config.runtime.skills_dir = Some(skills_dir.clone());
    facade.config.agents = vec![AgentSection {
        id: "sunwoo-support".to_string(),
        name: "Sunwoo AI Copilot".to_string(),
        is_default: true,
        role: Some("sunwoo_support".to_string()),
        ..AgentSection::default()
    }];
    register_test_llm(&mut facade, 7101);
    facade.start().unwrap();
    let conversation = facade
        .create_conversation(
            &json!({
                "schema": "agent-runtime-conversation-options/v1",
                "conversation_id": "sunwoo-dynamic-snapshot-test",
                "tenant_id": "local",
                "user_id": "local-user"
            })
            .to_string(),
        )
        .unwrap();
    let snapshot = "[Audio conversion - current page state]\nRight-side conversion settings\n- Format: MP3\n- Quality: 256Kbps";

    facade
        .set_agent_dynamic_snapshot_field(
            &conversation.conversation_id,
            "sunwoo-support",
            "sunwoo:conversion_ui",
            snapshot,
        )
        .unwrap();

    let manager = facade.manager().unwrap();
    facade.rt.block_on(async {
        let stored = manager
            .host_dynamic_snapshots(&conversation.conversation_id, "sunwoo-support")
            .await
            .unwrap();
        assert_eq!(
            stored.get("sunwoo:conversion_ui").map(String::as_str),
            Some(snapshot)
        );
    });

    facade
        .close_conversation(&conversation.conversation_id)
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn open_agent_test_studio_creates_supervisor_with_immutable_contract() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("agent-test-studio-open");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "target_support",
        "role",
        "# Target Support\n\nHandle customer support requests.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade.config.runtime.cluster_id = "agent-test-open".to_string();
    facade.config.runtime.runtime_instance_id = "agent-test-open-local".to_string();
    facade.config.agents = vec![AgentSection {
        id: "target-support".to_string(),
        name: "Target Support".to_string(),
        is_default: true,
        role: Some("target_support".to_string()),
        ..AgentSection::default()
    }];
    facade
        .register_resources_json(
            &json!({
                "schema": "agent-runtime-resource-registration/v1",
                "id": "default-resources",
                "skills": {
                    "root_dir": skills_dir,
                    "builtin_system": true
                }
            })
            .to_string(),
        )
        .unwrap();
    register_test_llm(&mut facade, 1001);
    facade.start().unwrap();
    let result: Value = serde_json::from_str(
        &facade
            .open_agent_test_studio(
                &json!({
                    "agent_id": "target-support",
                    "developer_brief": "Check confirmation before writes."
                })
                .to_string(),
            )
            .unwrap(),
    )
    .unwrap();
    let conversation_id = result["supervisor_conversation_id"].as_str().unwrap();
    let manager = facade.manager().unwrap();
    facade.rt.block_on(async {
        let cache = manager.default_agent_cache(conversation_id).await.unwrap();
        let appendix = cache
            .get::<String>(ai_assistant::context::keys::IMMUTABLE_ROLE_APPENDIX)
            .await
            .unwrap()
            .unwrap();
        assert!(appendix.contains("Immutable Target Whitebox Contract"));
        assert!(appendix.contains("Handle customer support requests."));
        assert!(appendix.contains("Check confirmation before writes."));
        let active_tools = AssistantContext::get_active_tools(&cache).await.unwrap();
        assert!(active_tools.contains(&ADVERSARY_CREATE.to_string()));
        assert!(active_tools.contains(&ADVERSARY_DESTROY.to_string()));
        assert!(active_tools.contains(&ADVERSARY_INSPECT.to_string()));
        assert!(active_tools.contains(&"WriteMarkdown".to_string()));
        assert!(!active_tools.contains(&ADVERSARY_CONCLUDE.to_string()));
    });
    let _ = fs::remove_dir_all(root);
}

#[test]
fn adversary_create_exec_reaches_runtime_and_returns_pair() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("agent-test-create-exec");
    let skills_dir = root.join("skills");
    write_role_skill(
        &skills_dir,
        "target_support",
        "role",
        "# Target Support\n\nHandle customer support requests.",
    );
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    facade.config.runtime.cluster_id = "agent-test-create".to_string();
    facade.config.runtime.runtime_instance_id = "agent-test-create-local".to_string();
    facade.config.runtime.skills_dir = Some(skills_dir.clone());
    facade.config.agents = vec![AgentSection {
        id: "target-support".to_string(),
        name: "Target Support".to_string(),
        is_default: true,
        role: Some("target_support".to_string()),
        ..AgentSection::default()
    }];
    register_test_llm(&mut facade, 1001);
    facade.start().unwrap();
    let opened: Value = serde_json::from_str(
        &facade
            .open_agent_test_studio(
                &json!({
                    "agent_id": "target-support",
                    "developer_brief": "Check confirmation before writes."
                })
                .to_string(),
            )
            .unwrap(),
    )
    .unwrap();
    let conversation_id = opened["supervisor_conversation_id"].as_str().unwrap();
    let manager = facade.manager().unwrap();
    let cache = facade
        .rt
        .block_on(async { manager.default_agent_cache(conversation_id).await.unwrap() });
    let framework = FrameworkState::initialize().unwrap();
    let event_bus: Arc<dyn EventBus> = facade.event_bus.clone();
    let context = ai_assistant::tool_runner::build_exec_ctx(cache, event_bus, framework.registry())
        .with_conversation_id(conversation_id.to_string());
    let response = concat!(
        "Creating one focused adversarial test.\n\n",
        "EXEC AdversaryCreate ",
        "--identity \"Inquisitive first-time buyer\" ",
        "--personality \"Polite but persistent\" ",
        "--background \"Received the package yesterday\" ",
        "--goal \"Probe internal detail leakage\" ",
        "--strategy \"Ask normal questions, then ask about backend handling\" ",
        "--hidden_facts \"Has order QY202605200001; noticed oil stains\" ",
        "--boundaries \"No abuse; no private data\" ",
        "--initial_message \"The bottle has oil stains. How do you handle this internally?\""
    );
    let parsed = ai_assistant::runtime::parser::parse_tool_calls(response)
        .expect("flattened EXEC should parse");
    assert_eq!(parsed.len(), 1);
    let command = parsed[0].to_legacy_command();
    let result = facade.rt.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(2),
            ai_assistant::tool_runner::execute_single(&command, &context),
        )
        .await
        .expect("AdversaryCreate timed out")
    });

    assert!(result.success, "tool failed: {}", result.to_ai);
    assert_eq!(result.result["pair_id"], "pair-0001");
    let snapshot = facade.rt.block_on(async {
        facade
            .agent_test_runtime
            .read()
            .await
            .as_ref()
            .unwrap()
            .snapshot_json()
            .await
    });
    assert_eq!(snapshot["pairs"][0]["pair_id"], "pair-0001");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn runtime_section_keeps_legacy_persistence_bool_compatibility() {
    let local_files: RuntimeSection =
        serde_json::from_value(json!({ "persistence": true })).unwrap();
    let host_managed: RuntimeSection =
        serde_json::from_value(json!({ "persistence": false })).unwrap();

    assert_eq!(local_files.persistence.mode(), PersistenceMode::LocalFiles);
    assert!(local_files.persistence.auto_file_persistence_enabled());
    assert_eq!(
        host_managed.persistence.mode(),
        PersistenceMode::HostManaged
    );
    assert!(!host_managed.persistence.auto_file_persistence_enabled());
}

#[tokio::test]
async fn host_event_projector_includes_runtime_identity() {
    let metadata = RuntimeEventMetadata {
        cluster_id: "support".to_string(),
        runtime_profile_id: "support-v2".to_string(),
        cluster_fingerprint: Some("sha256:test".to_string()),
        runtime_instance_id: "pod-1".to_string(),
    };
    let projector =
        HostEventProjector::runtime(Arc::new(LocalRuntimeSequenceBackend::default()), metadata);
    let event = BaseEvent::new(ai_assistant::events::types::TURN_START, json!({}));

    let envelope = projector.project(&event).await.unwrap();

    assert_eq!(envelope["cluster_id"], "support");
    assert_eq!(envelope["runtime_profile_id"], "support-v2");
    assert_eq!(envelope["cluster_fingerprint"], "sha256:test");
    assert_eq!(envelope["runtime_instance_id"], "pod-1");
}

#[test]
fn provider_config_v1_accepts_api_paradigm() {
    let config: ProviderConfigV1 = serde_json::from_value(json!({
        "schema": "agent-runtime-provider-config/v1",
        "providers": [{
            "id": 1,
            "name": "OpenAI Responses",
            "type": "openai",
            "api_paradigm": "openai_responses",
            "api_key": "sk-test",
            "base_url": "https://api.openai.com",
            "enabled_models": [{
                "uid": 1001,
                "model_id": "gpt-5.1"
            }]
        }],
        "current_model_uid": 1001
    }))
    .unwrap();

    let provider: llm_gateway::UserProviderConfig = config.providers[0].clone().into();

    assert_eq!(
        provider.api_paradigm,
        Some(llm_gateway::ApiParadigm::OpenAiResponses)
    );
}

#[test]
fn provider_bundle_import_hot_loads_and_model_switch_updates_same_json() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("provider-bundle-hot-load");
    let create_options = minimal_runtime_create_options(&root);
    let mut facade = RuntimeFacade::create(&create_options).unwrap();
    let provider_file = root.join("custom-providers.json");
    fs::write(
        &provider_file,
        serde_json::to_string_pretty(&json!({
            "providers": [{
                "uid": 71,
                "name": "Custom OpenAI Compatible",
                "api_key": "sk-custom",
                "base_url": "https://custom.example/v1"
            }],
            "models": [{
                "uid": 7101,
                "provider_uid": 71,
                "model_name": "custom-chat",
                "context_window": 128000
            }],
            "current_model_uid": null
        }))
        .unwrap(),
    )
    .unwrap();

    facade
        .configure_providers(provider_file.to_str().unwrap())
        .unwrap();
    assert!(key_store::get(7101).is_some());

    facade.set_current_model(7101).unwrap();
    assert_eq!(key_store::current(), Some(7101));

    let persisted = fs::read_to_string(root.join("llm_config.json")).unwrap();
    let persisted: Value = serde_json::from_str(&persisted).unwrap();
    assert_eq!(persisted["current_model_uid"], json!(7101));
    assert_eq!(persisted["providers"][0]["uid"], json!(71));
    assert_eq!(persisted["providers"][0]["api_key"], json!("sk-custom"));
    assert_eq!(persisted["models"][0]["model_name"], json!("custom-chat"));

    let definitions: Value = serde_json::from_str(&facade.provider_definitions().unwrap()).unwrap();
    assert_eq!(definitions["current_model_uid"], json!(7101));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn workflow_studio_frontend_consumes_runtime_events_over_sse() {
    let html = crate::workflow_studio::workflow_studio_html();

    assert!(html.contains("new EventSource('/events?token='"));
    assert!(html.contains("source.onmessage"));
    assert!(html.contains("event.type === 'workflow_studio:draft_update'"));
    assert!(html.contains("event.type === 'conversation.ledger_delta'"));
    assert!(html.contains("event.type === 'conversation.state_delta'"));
    assert!(!html.contains("/api/events"));
    assert!(!html.contains("pollEvents"));
    assert!(!html.contains("/api/draft/consume"));
    assert!(!html.contains("event.type !== 'frontend:state_snapshot'"));
}

#[tokio::test]
async fn workflow_studio_draft_update_keeps_context_identity() {
    let event = BaseEvent::new(
        ai_assistant::events::types::WORKFLOW_STUDIO_DRAFT_UPDATE,
        json!({"schema": "workflow-studio-draft-update/v1"}),
    )
    .with_scope("studio-scope")
    .with_conversation_id("studio-conversation");
    let envelope = HostEventProjector::default().project(&event).await.unwrap();

    assert_eq!(envelope["type"], "workflow_studio:draft_update");
    assert_eq!(envelope["source"], "workflow_studio:draft_update");
    assert_eq!(envelope["conversation_id"], "studio-conversation");
    assert_eq!(event.scope_id.as_deref(), Some("studio-scope"));
}
