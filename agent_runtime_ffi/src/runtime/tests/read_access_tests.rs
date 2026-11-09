use super::*;
use ai_assistant::systems::{grep::GrepSystem, grep_json::GrepJsonSystem};
use corework::ai_system::AIInput;
use corework::execution_unit::{ExecutionUnit, UnitType};
use corework::system::SystemOperation;

#[test]
fn conversation_read_access_ffi_enforces_cluster_ceiling_and_isolation() {
    let _guard = runtime_start_test_guard();
    let root = unique_test_dir("read-access-ffi");
    let mut facade = dynamic_cluster_runtime(&root);
    for project in ["a", "b"] {
        fs::create_dir_all(root.join("projects").join(project)).unwrap();
        fs::write(
            root.join("projects").join(project).join("data.json"),
            format!(r#"{{"project":"{project}"}}"#),
        )
        .unwrap();
    }
    facade
        .register_resources_json(
            &json!({
                "id": "search-resources", "skills": { "root_dir": root.join("skills") },
                "grep": {
                    "require_conversation_grants": true,
                    "roots": [
                        { "id": "project", "path": root.join("projects") },
                        { "id": "private", "path": root.join("skills") }
                    ]
                }
            })
            .to_string(),
        )
        .unwrap();
    facade.start().unwrap();
    let mut cluster = dynamic_cluster_registration("search", "cluster_role_a");
    cluster["grep_root_ids"] = json!(["project"]);
    facade.add_agent_cluster(&cluster).unwrap();
    let spawn = json!({ "cluster_id": "search" }).to_string();
    let a = facade.spawn_conversation(&spawn).unwrap();
    let b = facade.spawn_conversation(&spawn).unwrap();
    let manager = facade.manager().unwrap();
    let unit = Arc::new(ExecutionUnit::new_root_in_scope(
        UnitType::Module,
        corework::world::FrameworkState::initialize().unwrap(),
        format!("read-access-{}", uuid::Uuid::new_v4()),
    ));
    let scoped = facade
        .rt
        .block_on(manager.grep_service(&a.conversation_id))
        .unwrap();
    unit.attach_shared_component(scoped).unwrap();
    let mut ctx = unit.create_context();
    ctx.conversation_id = Some(a.conversation_id.clone());
    let json_input = || {
        AIInput::from_args(HashMap::from([
            ("root_id".into(), "project".into()),
            ("path".into(), "data.json".into()),
            ("action".into(), "read".into()),
            // A model-provided identity cannot choose the authorization.
            ("conversation_id".into(), b.conversation_id.clone()),
        ]))
    };
    let initial = facade
        .rt
        .block_on(GrepJsonSystem.execute(json_input(), &ctx))
        .unwrap();
    assert!(
        initial.to_ai.contains("read_access_required"),
        "{}",
        initial.to_ai
    );
    let payload = json!({
        "conversation_id": a.conversation_id,
        "grants": [{ "root_id": "project", "subpath": "a" }]
    });
    crate::invoke_command(
        &mut facade,
        "conversation.set_read_access",
        payload.as_object().unwrap(),
        "grant-a",
    )
    .unwrap();
    let result = facade
        .rt
        .block_on(GrepJsonSystem.execute(json_input(), &ctx))
        .unwrap();
    assert_eq!(result.error_code, 0, "{}", result.to_ai);
    assert!(result.to_ai.contains("\"a\""));
    let grep = facade
        .rt
        .block_on(GrepSystem.execute(
            AIInput::from_args(HashMap::from([
                ("root_id".into(), "project".into()),
                ("pattern".into(), "project".into()),
            ])),
            &ctx,
        ))
        .unwrap();
    assert_eq!(grep.error_code, 0, "{}", grep.to_ai);
    assert!(!grep.to_ai.contains("\"b\""));
    let bad = json!({ "conversation_id": a.conversation_id,
        "grants": [{ "root_id": "private", "subpath": "" }] });
    assert!(crate::invoke_command(
        &mut facade,
        "conversation.set_read_access",
        bad.as_object().unwrap(),
        "grant-private"
    )
    .is_err());
    let other = facade
        .rt
        .block_on(manager.grep_service(&b.conversation_id))
        .unwrap();
    assert!(other.set_read_access(&a.conversation_id, &[]).is_err());
    let snapshot = facade
        .export_conversation_snapshot(&a.conversation_id, "{}")
        .unwrap();
    facade
        .import_conversation_snapshot(
            &snapshot,
            &json!({ "conversation_id": a.conversation_id }).to_string(),
        )
        .unwrap();
    let after_restore = facade
        .rt
        .block_on(GrepJsonSystem.execute(json_input(), &ctx))
        .unwrap();
    assert_ne!(
        after_restore.error_code, 0,
        "restored history must not carry authorization"
    );
    crate::invoke_command(
        &mut facade,
        "conversation.set_read_access",
        payload.as_object().unwrap(),
        "regrant-after-restore",
    )
    .unwrap();
    let revoke = json!({ "conversation_id": a.conversation_id });
    crate::invoke_command(
        &mut facade,
        "conversation.revoke_read_access",
        revoke.as_object().unwrap(),
        "revoke-a",
    )
    .unwrap();
    let denied = facade
        .rt
        .block_on(GrepJsonSystem.execute(json_input(), &ctx))
        .unwrap();
    assert_ne!(denied.error_code, 0);
    facade.close_conversation(&a.conversation_id).unwrap();
    assert!(crate::invoke_command(
        &mut facade,
        "conversation.set_read_access",
        payload.as_object().unwrap(),
        "grant-closed"
    )
    .is_err());
    facade.close_conversation(&b.conversation_id).unwrap();
    drop(facade);
    let _ = fs::remove_dir_all(root);
}
