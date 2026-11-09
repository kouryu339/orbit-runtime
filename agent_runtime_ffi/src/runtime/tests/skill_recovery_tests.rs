use super::*;
use corework::cache::CacheExt;
use std::io::{BufRead, Read};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Probe(Arc<AtomicUsize>);
#[async_trait]
impl DynamicExecute for Probe {
    async fn execute_dynamic(
        &self,
        _: HashMap<String, Value>,
        _: &corework::orchestration::Context,
    ) -> corework::error::Result<Value> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"error_code":0,"to_ai":"PROBE_RESULT","result":{"ok":true}}))
    }
}

fn setup(root: &Path, body: &str) -> (RuntimeFacade, Arc<AtomicUsize>) {
    write_role_skill(&root.join("skills"), "recovery-role", "role", body);
    let role = root.join("skills/role/recovery-role/SKILL.md");
    let source = fs::read_to_string(&role).unwrap().replacen(
        "kind: role",
        "kind: role\ntools: [RecoveryProbe, RecoveryRead]",
        1,
    );
    fs::write(role, source).unwrap();
    write_role_skill(
        &root.join("skills"),
        "recovery-capability",
        "capability",
        "CURRENT_CAPABILITY_BODY",
    );
    let mut facade = RuntimeFacade::create(&minimal_runtime_create_options(root)).unwrap();
    facade
        .register_resources_json(
            &json!({"id":"recovery-resources","skills":{"root_dir":root.join("skills")}})
                .to_string(),
        )
        .unwrap();
    register_test_llm(&mut facade, 7301);
    facade.register_agent_cluster_json(&json!({
        "id":"recover-team","description":"Skill recovery fixture","focus_agent_id":"primary","agents":[{"id":"primary","role":"recovery-role","features":["recovery-capability"],"system_skills":{"thinking":"thinking-pro"},"tool_protocol":"native_fc"}]
    }).to_string()).unwrap();
    facade.start().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    for (name, readonly) in [("RecoveryProbe", false), ("RecoveryRead", true)] {
        let metadata=serde_json::from_value(json!({"name":name,"readonly":readonly,"destructive":false,"agent_enabled":true,"endpoint_id":"fixture","service":"fixture","method":"execute","parameters":[{"name":"value","param_type":"String","required":true}]})).unwrap();
        facade
            .tool_registry
            .register_dynamic_with_metadata(metadata, Arc::new(Probe(calls.clone())));
        facade.runtime_tools.push(
            facade
                .tool_registry
                .get_dynamic_metadata(name)
                .unwrap()
                .clone(),
        );
    }
    (facade, calls)
}

fn history() -> Vec<Value> {
    let mut records = Vec::new();
    let mut add = |role: &str, content: &str, metadata: Value| {
        records.push(json!({"record_id":records.len()+1,"conversation_id":"original","agent_id":"primary","agent_name":"Employee","role":role,"content":content,"metadata":metadata,"created_at":"2026-11-09T10:00:00+08:00"}));
    };
    add("user", "ORIGINAL_USER_MESSAGE", json!({}));
    add(
        "assistant",
        "ORIGINAL_AI_RAW",
        json!({"extra":{"tool_protocol":"native_fc","tool_calls":[
            {"id":"z-first","type":"function","function":{"name":"RecoveryRead","arguments":"{\"value\":\"first\"}"}},
            {"id":"a-second","type":"function","function":{"name":"RecoveryRead","arguments":"{\"value\":\"second\"}"}}
        ]}}),
    );
    for (id, result) in [
        ("z-first", "FIRST_ORIGINAL_RESULT"),
        ("a-second", "SECOND_ORIGINAL_RESULT"),
    ] {
        add(
            "tool",
            result,
            json!({"subtype":"tool_call_finished","tool_name":"RecoveryRead","success":true,"extra":{"call_id":id,"tool_call_id":id,"status":"success"}}),
        );
    }
    add("assistant", "ORIGINAL_FINAL_RAW", json!({}));
    records
}

fn archived(facade: &mut RuntimeFacade) -> Value {
    let info = facade
        .spawn_conversation(r#"{"cluster_id":"recover-team"}"#)
        .unwrap();
    let mut snapshot: Value = serde_json::from_str(
        &facade
            .export_conversation_snapshot(&info.conversation_id, "{}")
            .unwrap(),
    )
    .unwrap();
    snapshot["ledger"] = json!(history());
    snapshot["state_deltas"] = json!([{"op":"agent_skills.set","agent_id":"primary","main_skills":["obsolete-role","deleted-feature"],"imported_skills":["recovery-capability","deleted-capability"],"active_tools":["OldPrivilegedTool"]}]);
    facade.close_conversation(&info.conversation_id).unwrap();
    snapshot
}

fn restored_ledger(facade: &RuntimeFacade, id: &str) -> Vec<Value> {
    facade
        .rt
        .block_on(facade.manager().unwrap().ledger(id, Default::default()))
        .unwrap()
        .into_iter()
        .map(|r| serde_json::to_value(r).unwrap())
        .collect()
}

fn local_model(facade: &mut RuntimeFacade) -> std_mpsc::Receiver<Value> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let (tx, rx) = std_mpsc::channel();
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return,
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = std::io::BufReader::new(&stream);
        let mut size = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-length") {
                    size = value.trim().parse::<usize>().unwrap();
                }
            }
        }
        let mut body = vec![0; size];
        reader.read_exact(&mut body).unwrap();
        drop(reader);
        let request: Value = serde_json::from_slice(&body).unwrap();
        let stream_mode = request["stream"] == true;
        tx.send(request).unwrap();
        let (kind, body) = if stream_mode {
            let delta = json!({"id":"recovery-fixture","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":"RECOVERY_NEXT_RESPONSE"},"finish_reason":null}]});
            let stop = json!({"id":"recovery-fixture","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
            (
                "text/event-stream",
                format!("data: {delta}\n\ndata: {stop}\n\ndata: [DONE]\n\n"),
            )
        } else {
            ("application/json",json!({"id":"recovery-fixture","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"RECOVERY_NEXT_RESPONSE"},"finish_reason":"stop"}]}).to_string())
        };
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    });
    facade.configure_providers(&json!({"schema":"agent-runtime-provider-config/v1","current_model_uid":7301,"providers":[{"id":1,"name":"recovery-local","type":"openai","api_key":"fixture","base_url":endpoint,"api_paradigm":"openai_chat_completions","enabled_models":[{"uid":7301,"model_id":"gpt-4o"}]}]}).to_string()).unwrap();
    rx
}

fn assert_original_history(facade: &RuntimeFacade, id: &str) {
    let actual = restored_ledger(facade, id);
    let expected = history();
    assert!(actual.len() >= expected.len());
    for (actual, mut expected) in actual.iter().zip(expected) {
        expected["conversation_id"] = json!(id);
        // Deserialize supplies defaults to metadata without changing any original value.
        let expected: ai_assistant::ledger::LedgerRecord =
            serde_json::from_value(expected).unwrap();
        assert_eq!(*actual, serde_json::to_value(expected).unwrap());
    }
    let typed = facade
        .rt
        .block_on(facade.manager().unwrap().ledger(id, Default::default()))
        .unwrap();
    let messages = ai_assistant::ledger::agent_context(&typed, "primary");
    assert_eq!(messages[1].content, "ORIGINAL_AI_RAW");
    assert_eq!(
        messages[1]
            .tool_calls
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        vec!["z-first", "a-second"]
    );
    assert_eq!(messages[2].tool_call_id.as_deref(), Some("z-first"));
    assert_eq!(messages[3].tool_call_id.as_deref(), Some("a-second"));
}

#[test]
fn upgrade_restores_nonempty_history_with_latest_skills_and_no_cached_authority() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("upgrade-recovery");
    let (mut facade, calls) = setup(&root, "ROLE_VERSION_ONE");
    let snapshot = archived(&mut facade);
    let current = facade.read_skill("recovery-role").unwrap();
    let mut request = json!({"expected_store_revision":0,"skills":[{"name":"recovery-role","kind":"role","expected_version":current["version"],"source":"---\nname: recovery-role\ndescription: Employee\nkind: role\ntools: [RecoveryRead]\n---\nROLE_VERSION_TWO"}]});
    let accepted = facade.skill_transaction(&request, true).unwrap();
    let result = crate::invoke_command(
        &mut facade,
        "conversation.spawn_from_snapshot",
        json!({"spawn":{"cluster_id":"recover-team"},"snapshot":snapshot})
            .as_object()
            .unwrap(),
        "restore-upgrade",
    )
    .unwrap();
    let id = result["conversation_id"].as_str().unwrap();
    assert_original_history(&facade, id);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "Completed calls must never execute again"
    );
    let report = &result["recovery"];
    assert_eq!(
        report["skill_versions"]["recovery-role"],
        accepted["skills"][0]["version"]
    );
    assert!(report["removed_optional_skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["name"] == "deleted-capability"));
    let cache = facade
        .rt
        .block_on(facade.manager().unwrap().default_agent_cache(id))
        .unwrap();
    let tools = facade
        .rt
        .block_on(AssistantContext::get_active_tools(&cache))
        .unwrap();
    assert!(tools.iter().any(|t| t == "RecoveryRead"));
    assert!(!tools
        .iter()
        .any(|t| t == "RecoveryProbe" || t == "OldPrivilegedTool"));
    let latest: Value =
        serde_json::from_str(&facade.export_conversation_snapshot(id, "{}").unwrap()).unwrap();
    assert_eq!(
        latest["skill_versions"]["recovery-role"],
        accepted["skills"][0]["version"]
    );
    assert!(!latest["skill_versions"]
        .as_object()
        .unwrap()
        .contains_key("deleted-capability"));
    let model = local_model(&mut facade);
    facade
        .send_message(id, "CONTINUE_ORIGINAL_HISTORY")
        .unwrap();
    let model_request = model.recv_timeout(Duration::from_secs(5)).unwrap();
    let messages = model_request["messages"].as_array().unwrap();
    let system = messages
        .iter()
        .filter(|m| m["role"] == "system")
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(system.contains("ROLE_VERSION_TWO"), "{system}");
    assert!(!system.contains("ROLE_VERSION_ONE"));
    assert!(system.contains("CURRENT_CAPABILITY_BODY"));
    let original = messages
        .iter()
        .find(|m| m["content"] == "ORIGINAL_AI_RAW")
        .unwrap();
    assert_eq!(original["tool_calls"][0]["id"], "z-first");
    assert_eq!(original["tool_calls"][1]["id"], "a-second");
    assert!(messages.iter().any(|m| m["role"] == "tool"
        && m["tool_call_id"] == "z-first"
        && m["content"] == "FIRST_ORIGINAL_RESULT"));
    facade.close_conversation(id).unwrap();
    request["expected_store_revision"] = accepted["store_revision"].clone();
    assert!(
        matches!(
            facade.skill_transaction(&request, true),
            Err(RuntimeError::SkillVersionConflict { .. })
        ),
        "Save conflicts remain enforced"
    );
    facade.shutdown().unwrap();
}

fn unfinished_snapshot(
    facade: &mut RuntimeFacade,
    tool: &str,
    args: &str,
    started: bool,
    paused: bool,
) -> Value {
    let mut snapshot = archived(facade);
    let call = json!({"id":"pending-provider-id","type":"function","function":{"name":tool,"arguments":args}});
    let mut records = history();
    records.truncate(1);
    let mut assistant = history()[1].clone();
    assistant["metadata"]["extra"]["tool_calls"] = json!([call]);
    records.push(assistant);
    if started {
        let mut fact = history()[2].clone();
        fact["record_id"] = json!(3);
        fact["role"] = json!("gateway_message");
        fact["content"] = json!("Already started");
        fact["metadata"] = json!({"subtype":"tool_call_started","tool_name":tool,"tool_command":format!("{tool} --value x"),"extra":{"call_id":"pending-provider-id","status":"running"}});
        records.push(fact);
    }
    snapshot["ledger"] = json!(records);
    snapshot["paused_agents"] = if paused {
        json!(["primary"])
    } else {
        json!([])
    };
    snapshot["state_deltas"] = json!([]);
    snapshot
}

#[test]
fn recovery_checks_current_permissions_contract_and_indeterminate_calls() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("recovery-policy");
    let (mut facade, calls) = setup(&root, "ROLE_CURRENT");
    for (tool, args, started, deny, reason) in [
        (
            "RecoveryProbe",
            r#"{"value":"x"}"#,
            false,
            true,
            "current_permission_denied",
        ),
        (
            "RecoveryRead",
            r#"{"old_parameter":"x"}"#,
            false,
            false,
            "Invalid parameters",
        ),
        (
            "RecoveryProbe",
            r#"{"value":"x"}"#,
            true,
            false,
            "interrupted_outcome_unknown",
        ),
    ] {
        let snapshot = unfinished_snapshot(&mut facade, tool, args, started, false);
        let permissions = if deny {
            json!({"controlled_change":"deny"})
        } else {
            json!({})
        };
        let result=crate::invoke_command(&mut facade,"conversation.spawn_from_snapshot",json!({"spawn":{"cluster_id":"recover-team","permissions":permissions},"snapshot":snapshot}).as_object().unwrap(),"restore-policy").unwrap();
        let id = result["conversation_id"].as_str().unwrap();
        assert!(
            result["recovery"]["blocked_calls"][0]["reason"]
                .as_str()
                .unwrap()
                .contains(reason),
            "{result}"
        );
        let records = restored_ledger(&facade, id);
        assert_eq!(
            records
                .iter()
                .filter(|r| r["role"] == "tool"
                    && r["metadata"]["extra"]["call_id"] == "pending-provider-id")
                .count(),
            1
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        facade.close_conversation(id).unwrap();
    }
    facade.shutdown().unwrap();
}

#[test]
fn paused_restore_keeps_resume_and_rechecks_approval() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("recovery-paused-approval");
    let (mut facade, calls) = setup(&root, "ROLE_CURRENT");
    let snapshot = unfinished_snapshot(
        &mut facade,
        "RecoveryProbe",
        r#"{"value":"x"}"#,
        false,
        true,
    );
    let info = facade
        .spawn_conversation_from_snapshot(
            &json!({"cluster_id":"recover-team","permissions":{"controlled_change":"ask"}})
                .to_string(),
            &snapshot.to_string(),
        )
        .unwrap();
    let id = &info.conversation_id;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let cache = facade
        .rt
        .block_on(facade.manager().unwrap().default_agent_cache(id))
        .unwrap();
    assert_eq!(
        facade
            .rt
            .block_on(cache.get::<String>(ai_assistant::context::keys::TASK_STATUS))
            .unwrap()
            .as_deref(),
        Some("paused")
    );
    assert!(facade
        .resume_conversation_with_admission(id, "resume-recovered".into())
        .unwrap()
        .decision
        .is_accepted());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let pending = facade.rt.block_on(async {
            // Inspect the authoritative emitted fact, never an archived decision.
            facade
                .manager()
                .unwrap()
                .ledger(id, Default::default())
                .await
                .unwrap()
        });
        if pending.iter().any(|r| {
            r.metadata.subtype.as_deref()
                == Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_PERMISSION_REQUESTED)
        }) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(facade
        .resolve_tool_permission(
            id,
            "pending-provider-id",
            ai_assistant::ToolPermissionDecision::Deny
        )
        .unwrap());
    facade.close_conversation(id).unwrap();
    facade.shutdown().unwrap();
}

#[test]
fn native_open_call_order_is_provider_order_not_lexicographic_id_order() {
    let mut records = history();
    records.truncate(2);
    let records: Vec<ai_assistant::ledger::LedgerRecord> =
        serde_json::from_value(json!(records)).unwrap();
    let plans = restored_execution_plans(&records);
    assert_eq!(plans[0].call_ids, vec!["z-first", "a-second"]);
    assert_eq!(plans[0].structured_tools[0].params[0].1, "first");
}

#[test]
fn required_skills_missing_preserve_existing_history_and_report_the_name() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("recovery-required-skills");
    let (mut facade, _) = setup(&root, "CURRENT_ROLE");
    let snapshot = archived(&mut facade);
    let info = facade
        .spawn_conversation_from_snapshot(r#"{"cluster_id":"recover-team"}"#, &snapshot.to_string())
        .unwrap();
    let options = json!({"conversation_id":info.conversation_id}).to_string();
    for system in [false, true] {
        let definition = &mut facade
            .registries
            .agent_clusters
            .get_mut("recover-team")
            .unwrap()
            .agents[0];
        definition.role = Some(
            if system {
                "recovery-role"
            } else {
                "missing-required-role"
            }
            .into(),
        );
        if system {
            definition
                .system_skills
                .insert("thinking".into(), "missing-required-thinking".into());
        }
        let error = facade
            .import_conversation_snapshot(&snapshot.to_string(), &options)
            .unwrap_err()
            .to_string();
        assert!(error.contains("required_skill_missing"), "{error}");
        assert!(
            error.contains(if system {
                "missing-required-thinking"
            } else {
                "missing-required-role"
            }),
            "{error}"
        );
        assert_original_history(&facade, &info.conversation_id);
    }
    facade.close_conversation(&info.conversation_id).unwrap();
    facade.shutdown().unwrap();
}

#[test]
fn skill_recovery_process_worker() {
    let Ok(stage) = std::env::var("ORBIT_SKILL_RECOVERY_STAGE") else {
        return;
    };
    let root = PathBuf::from(std::env::var("ORBIT_SKILL_RECOVERY_ROOT").unwrap());
    let (mut facade, calls) = setup(
        &root,
        if stage == "export" {
            "CROSS_PROCESS_ROLE_ONE"
        } else {
            "CROSS_PROCESS_ROLE_TWO"
        },
    );
    if stage == "export" {
        let snapshot = archived(&mut facade);
        fs::write(root.join("checkpoint.json"), snapshot.to_string()).unwrap();
    } else {
        let snapshot = fs::read_to_string(root.join("checkpoint.json")).unwrap();
        let info = facade
            .spawn_conversation_from_snapshot(r#"{"cluster_id":"recover-team"}"#, &snapshot)
            .unwrap();
        assert_original_history(&facade, &info.conversation_id);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let exported: Value = serde_json::from_str(
            &facade
                .export_conversation_snapshot(&info.conversation_id, "{}")
                .unwrap(),
        )
        .unwrap();
        assert_ne!(
            exported["skill_versions"]["recovery-role"],
            serde_json::from_str::<Value>(&snapshot).unwrap()["skill_versions"]["recovery-role"]
        );
        assert!(cached_skill_body(&facade, "recovery-role").contains("CROSS_PROCESS_ROLE_TWO"));
        let model = local_model(&mut facade);
        facade
            .send_message(&info.conversation_id, "CROSS_PROCESS_CONTINUE")
            .unwrap();
        let request = model.recv_timeout(Duration::from_secs(5)).unwrap();
        let system = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "system")
            .map(|m| m["content"].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(system.contains("CROSS_PROCESS_ROLE_TWO"));
        assert!(!system.contains("CROSS_PROCESS_ROLE_ONE"));
        assert!(request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["tool_call_id"] == "a-second"));
        fs::write(root.join("recovered.json"), exported.to_string()).unwrap();
        facade.close_conversation(&info.conversation_id).unwrap();
    }
    facade.shutdown().unwrap();
}

#[test]
fn skill_upgrade_recovers_the_same_history_across_processes() {
    let root = unique_test_dir("cross-process-skill-upgrade");
    for stage in ["export", "restore"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::tests::skill_recovery_tests::skill_recovery_process_worker",
                "--nocapture",
            ])
            .env("ORBIT_SKILL_RECOVERY_STAGE", stage)
            .env("ORBIT_SKILL_RECOVERY_ROOT", &root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stage={stage}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let result: Value =
        serde_json::from_slice(&fs::read(root.join("recovered.json")).unwrap()).unwrap();
    assert_eq!(result["ledger"].as_array().unwrap().len(), 5);
}
