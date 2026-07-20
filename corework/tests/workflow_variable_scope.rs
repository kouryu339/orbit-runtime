use std::collections::HashMap;

use corework::workflow::blueprint_loader::BlueprintLoader;
use corework::workflow::chain_compiler_v2::compile_chain_v2;
use corework::workflow::execution::ExecutionContext;
use corework::world::FrameworkState;

#[tokio::test]
async fn get_var_node_reads_declared_variable_default_from_scope() {
    let json = r#"
    {
        "version": "1.0",
        "metadata": {
            "name": "get_var_default",
            "created": "2024-01-01T00:00:00Z",
            "modified": "2024-01-01T00:00:00Z"
        },
        "variables": [
            {"name": "label", "data_type": "String", "default_value": "hello"}
        ],
        "nodes": [
            {"id": "start_1", "node_type": "StartNode", "pins": [{"name": "Out", "kind": "ExecOutput"}]},
            {
                "id": "get_label",
                "node_type": "GetVarNode",
                "pins": [{"name": "Value", "kind": "DataOutput", "data_type": "Any"}],
                "properties": {"variable_name": "label"}
            },
            {
                "id": "end_1",
                "node_type": "EndNode",
                "pins": [
                    {"name": "In", "kind": "ExecInput"},
                    {"name": "result", "kind": "DataInput", "data_type": "Any"}
                ]
            }
        ],
        "connections": [
            {"source_node": "start_1", "source_pin": "Out", "target_node": "end_1", "target_pin": "In", "connection_type": "Exec"},
            {"source_node": "get_label", "source_pin": "Value", "target_node": "end_1", "target_pin": "result", "connection_type": "Data"}
        ]
    }
    "#;

    let ctx = FrameworkState::initialize().unwrap().create_context();
    let loaded = BlueprintLoader::new()
        .load_from_json_str(json, &ctx)
        .unwrap();
    let mut exec_ctx = ExecutionContext::from_context(ctx);
    loaded
        .compiled
        .initialize_defaults(&mut exec_ctx)
        .await
        .unwrap();
    let outputs = loaded
        .compiled
        .executor()
        .execute_with_params(&mut exec_ctx, HashMap::new())
        .await
        .unwrap();

    assert_eq!(
        outputs.get("result").and_then(|value| value.as_str()),
        Some("hello")
    );
}

#[test]
fn get_var_node_rejects_undeclared_variable_in_loader() {
    let json = r#"
    {
        "version": "1.0",
        "metadata": {
            "name": "get_var_reject",
            "created": "2024-01-01T00:00:00Z",
            "modified": "2024-01-01T00:00:00Z"
        },
        "nodes": [
            {"id": "start_1", "node_type": "StartNode", "pins": [{"name": "Out", "kind": "ExecOutput"}]},
            {
                "id": "get_missing",
                "node_type": "GetVarNode",
                "pins": [{"name": "Value", "kind": "DataOutput", "data_type": "Any"}],
                "properties": {"variable_name": "missing"}
            },
            {"id": "end_1", "node_type": "EndNode", "pins": [{"name": "In", "kind": "ExecInput"}]}
        ],
        "connections": [
            {"source_node": "start_1", "source_pin": "Out", "target_node": "end_1", "target_pin": "In", "connection_type": "Exec"}
        ]
    }
    "#;

    let ctx = FrameworkState::initialize().unwrap().create_context();
    let error = BlueprintLoader::new()
        .load_from_json_str(json, &ctx)
        .expect_err("undeclared GetVarNode reference should fail");
    assert!(error.to_string().contains("GetVarNode"));
}

#[test]
fn setvar_rejects_undeclared_variable_at_compile_time() {
    let error = compile_chain_v2(
        r#"
INPUT
1: setvar $missing = "value"
RETURN result="done"
"#,
    )
    .expect_err("undeclared SetVar target should fail in compiler");

    assert!(error.to_string().contains("SetVarNode"));
}

#[test]
fn setvar_rejects_loop_builtin_item_at_compile_time() {
    let error = compile_chain_v2(
        r#"
input strings:Array[String]
1: FOR input.strings
    2.1: setvar item = "b"
END
RETURN result=input.strings
"#,
    )
    .expect_err("loop builtin item should not be writable by SetVar");

    let error_text = error.to_string();
    assert!(error_text.contains("SetVarNode"));
}

#[test]
fn variable_declaration_does_not_generate_a_node_when_unused() {
    let blueprint = compile_chain_v2(
        r#"
INPUT
$label = "hello"
RETURN result="done"
"#,
    )
    .unwrap();

    assert_eq!(blueprint.variables.len(), 1);
    assert_eq!(blueprint.variables[0].name, "label");
    assert!(blueprint
        .nodes
        .iter()
        .all(|node| node.node_type != "GetVarNode" && node.node_type != "SetVarNode"));
}

#[test]
fn variable_reference_generates_one_reusable_get_var_node() {
    let blueprint = compile_chain_v2(
        r#"
INPUT
$label = "hello"
RETURN first=$label second=$label
"#,
    )
    .unwrap();

    assert_eq!(blueprint.variables.len(), 1);
    assert_eq!(
        blueprint
            .nodes
            .iter()
            .filter(|node| node.node_type == "GetVarNode")
            .count(),
        1
    );
    let get_var = blueprint
        .nodes
        .iter()
        .find(|node| node.node_type == "GetVarNode")
        .unwrap();
    let name_pin = get_var
        .pins
        .iter()
        .find(|pin| pin.name == "Name" && pin.kind == "DataInput")
        .unwrap();
    assert_eq!(
        name_pin
            .default_value
            .as_ref()
            .and_then(|value| value.as_str()),
        Some("label")
    );
    let value_pin = get_var
        .pins
        .iter()
        .find(|pin| pin.name == "Value" && pin.kind == "DataOutput")
        .unwrap();
    assert_eq!(value_pin.data_type, "String");
}

#[tokio::test]
async fn connected_get_var_name_reads_a_declared_variable() {
    let blueprint = compile_chain_v2(
        r#"
input variable_name:String
$label = "hello"
return result=getvar(input.variable_name)
"#,
    )
    .unwrap();
    let json = serde_json::to_string(&blueprint).unwrap();
    let ctx = FrameworkState::initialize().unwrap().create_context();
    let loaded = BlueprintLoader::new()
        .load_from_json_str(&json, &ctx)
        .unwrap();
    let mut exec_ctx = ExecutionContext::from_context(ctx);
    loaded
        .compiled
        .initialize_defaults(&mut exec_ctx)
        .await
        .unwrap();
    let outputs = loaded
        .compiled
        .executor()
        .execute_with_params(
            &mut exec_ctx,
            HashMap::from([(
                "variable_name".to_string(),
                corework::workflow::core::DataValue::from_string("label"),
            )]),
        )
        .await
        .unwrap();

    assert_eq!(
        outputs.get("result").and_then(|value| value.as_str()),
        Some("hello")
    );
}

#[test]
fn variable_declaration_rejects_runtime_references_and_expressions() {
    let invalid_initializers = [
        ("step output", "1.output"),
        ("workflow input", "input.source"),
        ("variable reference", "$source"),
        ("pure expression", "trim(input.source)"),
    ];

    for (case, initializer) in invalid_initializers {
        let script = format!(
            "input source:String\n$source = \"seed\"\n$alias = {}\nreturn result=input.source",
            initializer
        );
        let error = match compile_chain_v2(&script) {
            Ok(_) => panic!("{} initializer should be rejected", case),
            Err(error) => error,
        };

        assert_eq!(
            error.kind,
            corework::workflow::chain_compiler::ChainErrorKind::Syntax
        );
        assert!(error.message.contains("static literal"));
        assert!(error
            .suggestion
            .as_deref()
            .unwrap_or_default()
            .contains("N.Pin"));
    }
}
