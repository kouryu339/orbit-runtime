use corework::workflow::blueprint_loader::BlueprintLoader;
use corework::workflow::chain_compiler_v2::compile_chain_v2;
use corework::workflow::chain_decompiler::decompile_chain;
use corework::workflow::core::DataValue;
use corework::workflow::execution::ExecutionContext;
use corework::world::FrameworkState;
use std::collections::HashMap;

#[tokio::test]
async fn split_compiles_roundtrips_loads_and_executes() {
    for script in [
        r#"input value:String separators:Array<String>
return parts=split(input.value, input.separators)"#,
        r#"input value:String
return parts=split(input.value, ["-", ",", "，"])"#,
    ] {
        let blueprint = compile_chain_v2(script).unwrap();
        assert!(blueprint
            .nodes
            .iter()
            .any(|node| node.node_type == "SplitNode"));
        let text = decompile_chain(&blueprint).unwrap();
        assert!(text.contains("split("), "{text}");
        let roundtrip = compile_chain_v2(&text).unwrap();
        let ctx = FrameworkState::initialize().unwrap().create_context();
        let loaded = BlueprintLoader::new()
            .load_from_json_str(&serde_json::to_string(&roundtrip).unwrap(), &ctx)
            .unwrap();
        let mut exec = ExecutionContext::from_context(ctx);
        loaded
            .compiled
            .initialize_defaults(&mut exec)
            .await
            .unwrap();
        let mut params = HashMap::from([("value".into(), DataValue::from_string("aa-c,i，尾"))]);
        if script.contains("separators:Array") {
            params.insert(
                "separators".into(),
                DataValue::from_array(vec!["-", ",", "，"], "String"),
            );
        }
        let output = loaded
            .compiled
            .executor()
            .execute_with_params(&mut exec, params)
            .await
            .unwrap();
        assert_eq!(
            output["parts"].extract_array::<String>().unwrap(),
            vec!["aa", "c", "i", "尾"]
        );
    }
}
