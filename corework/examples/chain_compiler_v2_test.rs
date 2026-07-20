use corework::prelude::*;
use corework::workflow::blueprint_loader::BlueprintLoader;
use corework::workflow::chain_compiler_v2::compile_chain_v2;
use corework::workflow::execution::ExecutionContext;
use std::collections::HashMap;
use std::sync::Arc;

fn make_exec_ctx(framework: &FrameworkState) -> ExecutionContext {
    ExecutionContext::new(
        Arc::new(InMemoryCache::new()) as Arc<dyn Cache>,
        Arc::new(InMemoryEventBus::new()) as Arc<dyn EventBus>,
        framework.telemetry.clone() as Arc<dyn Telemetry>,
        framework.registry.clone(),
    )
}

async fn run_chain(
    chain_text: &str,
    inputs: HashMap<String, DataValue>,
    framework: &FrameworkState,
) -> Result<HashMap<String, DataValue>> {
    let bp = compile_chain_v2(chain_text)
        .map_err(|e| FrameworkError::WorkflowError(format!("compile_chain_v2 失败: {}", e)))?;

    let orch_ctx = framework.create_context();
    let loaded = BlueprintLoader::new()
        .load_from_blueprint_json(bp, &orch_ctx)
        .map_err(|e| FrameworkError::WorkflowError(format!("BlueprintLoader 失败: {}", e)))?;

    let mut exec_ctx = make_exec_ctx(framework);
    loaded.compiled.initialize_defaults(&mut exec_ctx).await?;

    let outputs = loaded
        .compiled
        .executor
        .clone()
        .execute_with_params(&mut exec_ctx, inputs)
        .await?;

    Ok(outputs)
}

async fn test_simple_add(framework: &FrameworkState) {
    let chain = r#"
INPUT

RETURN result=$(AddNode 3.0 4.0)
"#;

    let outputs = run_chain(chain, HashMap::new(), framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_simple_add] 执行失败: {}", e));

    let result = outputs
        .get("result")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| {
            panic!(
                "❌ [test_simple_add] 缺少 'result' 输出，实际: {:?}",
                outputs
            )
        });

    assert_eq!(
        result, 7.0,
        "❌ [test_simple_add] 期望 7.0，实际 {}",
        result
    );
    println!("✅ [test_simple_add] result = {}", result);
}

async fn test_chained_pure(framework: &FrameworkState) {
    let chain = r#"
INPUT

RETURN out=$(MultiplyNode $(AddNode 1.0 2.0) 2.0)
"#;

    let outputs = run_chain(chain, HashMap::new(), framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_chained_pure] 执行失败: {}", e));

    let out = outputs
        .get("out")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| {
            panic!(
                "❌ [test_chained_pure] 缺少 'out' 输出，实际: {:?}",
                outputs
            )
        });

    assert_eq!(out, 6.0, "❌ [test_chained_pure] 期望 6.0，实际 {}", out);
    println!("✅ [test_chained_pure] out = {}", out);
}

async fn test_input_params(framework: &FrameworkState) {
    let chain = r#"
INPUT x:Any y:Any

RETURN result=$(AddNode input.x input.y)
"#;

    let mut inputs = HashMap::new();
    inputs.insert("x".to_string(), DataValue::from_f64(15.0));
    inputs.insert("y".to_string(), DataValue::from_f64(27.0));

    let outputs = run_chain(chain, inputs, framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_input_params] 执行失败: {}", e));

    let result = outputs
        .get("result")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| {
            panic!(
                "❌ [test_input_params] 缺少 'result' 输出，实际: {:?}",
                outputs
            )
        });

    assert_eq!(
        result, 42.0,
        "❌ [test_input_params] 期望 42.0，实际 {}",
        result
    );
    println!("✅ [test_input_params] result = {}", result);
}

async fn test_for_loop(framework: &FrameworkState) {
    let chain = r#"
INPUT
$num = 0.0
FOR 1 TO 10
    1.1: SetVar $num $(MultiplyNode $index 1.0)
END
RETURN last=$num
"#;

    let outputs = run_chain(chain, HashMap::new(), framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_for_loop] 执行失败: {}", e));

    let last = outputs
        .get("last")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("❌ [test_for_loop] 缺少 'last' 输出，实际: {:?}", outputs));

    assert_eq!(last, 10.0, "❌ [test_for_loop] 期望 10.0，实际 {}", last);
    println!("✅ [test_for_loop] last = {}", last);
}

async fn test_for_loop_break(framework: &FrameworkState) {
    let chain = r#"
INPUT stop:Any=7
$num = 0.0
FOR 1 TO 10
    IF $(EqualNode input.stop $index)
        BREAK
    ELSE
        SetVar $num $(MultiplyNode $index 1.0)
    END
END
RETURN last=$num
"#;

    let outputs = run_chain(chain, HashMap::new(), framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_for_loop_break/default] 执行失败: {}", e));

    let last = outputs
        .get("last")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| {
            panic!(
                "❌ [test_for_loop_break/default] 缺少 'last'，实际: {:?}",
                outputs
            )
        });

    assert_eq!(
        last, 6.0,
        "❌ [test_for_loop_break/default] 期望 6.0，实际 {}",
        last
    );
    println!("✅ [test_for_loop_break] stop=7(default) last = {}", last);

    let mut inputs = HashMap::new();
    inputs.insert("stop".to_string(), DataValue::from_i64(4));

    let outputs = run_chain(chain, inputs, framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_for_loop_break/stop=4] 执行失败: {}", e));

    let last = outputs
        .get("last")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| {
            panic!(
                "❌ [test_for_loop_break/stop=4] 缺少 'last'，实际: {:?}",
                outputs
            )
        });

    assert_eq!(
        last, 3.0,
        "❌ [test_for_loop_break/stop=4] 期望 3.0，实际 {}",
        last
    );
    println!("✅ [test_for_loop_break] stop=4 last = {}", last);
}

async fn test_foreach_string_join(framework: &FrameworkState) {
    let chain = r#"
INPUT strarray:Array
$sentence = ""
FOR input.strarray
    1.1: SetVar $sentence $(StringAppendNode $(StringAppendNode $sentence " ") $item.Element)
END
RETURN sentence=$(TrimNode $sentence)
"#;

    let mut inputs = HashMap::new();
    inputs.insert(
        "strarray".to_string(),
        DataValue::from_array(
            vec!["hello".to_string(), "world".to_string(), "rust".to_string()],
            "String",
        ),
    );

    let outputs = run_chain(chain, inputs, framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_foreach_string_join/3words] 执行失败: {}", e));

    let sentence = outputs
        .get("sentence")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| {
            panic!(
                "❌ [test_foreach_string_join/3words] 缺少 'sentence'，实际: {:?}",
                outputs
            )
        });

    assert_eq!(
        sentence, "hello world rust",
        "❌ [test_foreach_string_join/3words] 期望 hello world rust，实际 {}",
        sentence
    );
    println!(
        "✅ [test_foreach_string_join] 3 words sentence = {:?}",
        sentence
    );

    let mut inputs = HashMap::new();
    inputs.insert(
        "strarray".to_string(),
        DataValue::from_array(vec!["foo".to_string(), "bar".to_string()], "String"),
    );

    let outputs = run_chain(chain, inputs, framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_foreach_string_join/2words] 执行失败: {}", e));

    let sentence = outputs
        .get("sentence")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| {
            panic!(
                "❌ [test_foreach_string_join/2words] 缺少 'sentence'，实际: {:?}",
                outputs
            )
        });

    assert_eq!(
        sentence, "foo bar",
        "❌ [test_foreach_string_join/2words] 期望 foo bar，实际 {}",
        sentence
    );
    println!(
        "✅ [test_foreach_string_join] 2 words sentence = {:?}",
        sentence
    );
}

async fn test_numbered_if(framework: &FrameworkState) {
    let chain = r#"
INPUT x:Any
$result = 0.0

1: IF $(EqualNode input.x 0.0)
    1.1: SetVar $result 0.0
ELSE
    2.1: SetVar $result input.x
END
RETURN result=$result
"#;

    let mut inputs = HashMap::new();
    inputs.insert("x".to_string(), DataValue::from_f64(5.0));
    let outputs = run_chain(chain, inputs, framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_numbered_if/x=5] 执行失败: {}", e));
    let result = outputs
        .get("result")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| {
            panic!(
                "❌ [test_numbered_if/x=5] 缺少 'result'，实际: {:?}",
                outputs
            )
        });
    assert_eq!(
        result, 5.0,
        "❌ [test_numbered_if/x=5] 期望 5.0，实际 {}",
        result
    );
    println!("✅ [test_numbered_if] x=5.0 result = {}", result);

    let mut inputs = HashMap::new();
    inputs.insert("x".to_string(), DataValue::from_f64(0.0));
    let outputs = run_chain(chain, inputs, framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_numbered_if/x=0] 执行失败: {}", e));
    let result = outputs
        .get("result")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| {
            panic!(
                "❌ [test_numbered_if/x=0] 缺少 'result'，实际: {:?}",
                outputs
            )
        });
    assert_eq!(
        result, 0.0,
        "❌ [test_numbered_if/x=0] 期望 0.0，实际 {}",
        result
    );
    println!("✅ [test_numbered_if] x=0.0 result = {}", result);
}

async fn test_numbered_for(framework: &FrameworkState) {
    let chain = r#"
INPUT
$sum = 0.0
1: FOR 1 TO 5
    1.1: SetVar $sum $(AddNode $sum $index)
END
RETURN total=$sum
"#;

    let outputs = run_chain(chain, HashMap::new(), framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_numbered_for] 执行失败: {}", e));

    let total = outputs
        .get("total")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("❌ [test_numbered_for] 缺少 'total'，实际: {:?}", outputs));

    assert_eq!(
        total, 15.0,
        "❌ [test_numbered_for] 期望 15.0，实际 {}",
        total
    );
    println!("✅ [test_numbered_for] total = {}", total);
}

async fn test_numbered_syntax_parse(_framework: &FrameworkState) {
    let chain = r#"
INPUT x:Any
$x = 0.0

1: IF $(EqualNode input.x 1.0)
    1.1: SetVar $x 1.0
2: ELIF $(EqualNode input.x 2.0)
    2.1: SetVar $x 2.0
ELSE
    3.1: SetVar $x 0.0
END
4: FOR 1 TO 3
    4.1: IF $(EqualNode $index 2)
        BREAK
    END
END
RETURN
"#;

    compile_chain_v2(chain)
        .unwrap_or_else(|e| panic!("❌ [test_numbered_syntax_parse] 解析失败: {}", e));
    println!("✅ [test_numbered_syntax_parse] N: IF/ELIF/ELSE/FOR 全部解析成功");
}

async fn test_second_line_empty(_framework: &FrameworkState) {
    let chain = r#"
INPUT

1: OpenBrowser --url "https://creator.douyin.com/"
RETURN
"#;

    let _bp = compile_chain_v2(chain)
        .map_err(|e| FrameworkError::WorkflowError(format!("compile_chain_v2 失败: {}", e)))
        .unwrap();
    println!("✅ [test_second_line_empty] 解析成功！第2行空行，第3行节点调用");
}

async fn test_second_line_node_call_should_succeed(_framework: &FrameworkState) {
    let chain = r#"
INPUT
OpenBrowser --url "https://creator.douyin.com/"
RETURN
"#;

    let _bp = compile_chain_v2(chain).unwrap_or_else(|e| {
        panic!(
            "❌ [test_second_line_node_call_should_succeed] v2 应允许第2行节点调用，但失败: {}",
            e
        )
    });
    println!("✅ [test_second_line_node_call_should_succeed] v2 允许第2行直接节点调用");
}

#[tokio::main]
async fn main() {
    println!("\n╔══════════════════════════════════════════════════════╗");
    println!("║        chain 编译器 v2 · 端到端执行测试             ║");
    println!("╚══════════════════════════════════════════════════════╝\n");

    let framework = FrameworkState::initialize().expect("FrameworkState 初始化失败");

    test_simple_add(&framework).await;
    test_chained_pure(&framework).await;
    test_input_params(&framework).await;
    test_for_loop(&framework).await;
    test_for_loop_break(&framework).await;
    test_foreach_string_join(&framework).await;
    test_numbered_if(&framework).await;
    test_numbered_for(&framework).await;
    test_numbered_syntax_parse(&framework).await;
    // test_numbered_assignment_rejected(); // 函数未在原 commit 中提交，暂跳过
    test_second_line_empty(&framework).await;
    test_second_line_node_call_should_succeed(&framework).await;

    println!("\n╔══════════════════════════════════════════════════════╗");
    println!("║  🎉  chain v2 全部 10 项测试通过！                  ║");
    println!("╚══════════════════════════════════════════════════════╝\n");
}
