//! chain 编译器端到端测试（E2E）
//!
//! 验证完整通路：链式文本 → compile_chain → BlueprintJson → BlueprintLoader
//! 合法性检查 + 实例化 → ExecutionContext → execute → 断言结果
//!
//! 使用的节点全部来自注册表：
//! - AddNode        (Pure):  A:f64, B:f64 → Result:f64
//! - MultiplyNode   (Pure):  A:f64, B:f64 → Result:f64
//! - EqualNode      (Pure):  A, B → Result:bool
//! - StringAppendNode(Pure): A:String, B:String → Result:String
//! - TrimNode       (Pure):  A:String → Result:String
//! - ForLoopNode  (Impure):  FirstIndex:i64, LastIndex:i64 → Index:i64；Exec: In/LoopBody/Completed
//! - ForEachNode  (Impure):  Array → Element/Index；Exec: In/LoopBody/Completed
//! - BreakNode    (Impure):  Exec: In
//! - SetVar       (Impure):  第一个位置参数为目标 $var，第二个为新值；Exec: In/Then
//!
//! 语法说明（新语法）：
//! - 步骤编号引用：N.pin（如 1.Result, 2.Element）— 步骤编号的指定输出引脚
//! - 控制流编号：N: IF / N: FOR / N: ELSE — IF/FOR/ELSE 也可带步骤编号
//! - 工作流入参：input.pin（如 input.x, input.y）— INPUT 声明的流入参数
//! - 变量：$var（如 $num, $index, $item）— 循环变量等可变状态
//! - 内联 Pure：$(NodeType(pin=value,...))— 内联数据处理节点

use corework::prelude::*;
use corework::workflow::blueprint_loader::BlueprintLoader;
use corework::workflow::chain_compiler::compile_chain;
use corework::workflow::execution::ExecutionContext;
use std::collections::HashMap;
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────────────
// 执行辅助
// ─────────────────────────────────────────────────────────────────────────────

/// 构建一个新鲜的 ExecutionContext（每次测试独立 cache）
fn make_exec_ctx(framework: &FrameworkState) -> ExecutionContext {
    ExecutionContext::new(
        Arc::new(InMemoryCache::new()) as Arc<dyn Cache>,
        Arc::new(InMemoryEventBus::new()) as Arc<dyn EventBus>,
        framework.telemetry.clone() as Arc<dyn Telemetry>,
        framework.registry.clone(),
    )
}

/// 完整执行管线：
/// chain_text → BlueprintJson → BlueprintLoader → initialize_defaults → execute
async fn run_chain(
    chain_text: &str,
    inputs: HashMap<String, DataValue>,
    framework: &FrameworkState,
) -> Result<HashMap<String, DataValue>> {
    // 1. 编译链式文本 → BlueprintJson（中间结构）
    let bp = compile_chain(chain_text)
        .map_err(|e| FrameworkError::WorkflowError(format!("compile_chain 失败: {}", e)))?;

    // 2. BlueprintLoader 对 JSON 做合法性检查 + 实例化
    let orch_ctx = framework.create_context();
    let loaded = BlueprintLoader::new()
        .load_from_blueprint_json(bp, &orch_ctx)
        .map_err(|e| FrameworkError::WorkflowError(format!("BlueprintLoader 失败: {}", e)))?;

    // 3. 初始化执行上下文
    let mut exec_ctx = make_exec_ctx(framework);

    // 4. 将 JSON 中的字面量默认值写入 cache
    loaded.compiled.initialize_defaults(&mut exec_ctx).await?;

    // 5. 执行工作流，返回 EndNode 收集的输出
    let outputs = loaded
        .compiled
        .executor
        .clone()
        .execute_with_params(&mut exec_ctx, inputs)
        .await?;

    Ok(outputs)
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1: 简单加法（流水线冒烟测试）
// ─────────────────────────────────────────────────────────────────────────────

/// StartNode → AddNode(A=3.0, B=4.0) → EndNode
/// 期望: result = 7.0
async fn test_simple_add(framework: &FrameworkState) {
    let chain = r#"
INPUT

RETURN result=$(AddNode(3.0, 4.0))
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
    println!("✅ [test_simple_add]  result = {}", result);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2: 链式 Pure 节点（Add 结果直接喂给 Multiply）
// ─────────────────────────────────────────────────────────────────────────────

/// $(AddNode(A=1.0, B=2.0)) = 3.0, $(MultiplyNode(A=3.0, B=2.0)) = 6.0
async fn test_chained_pure(framework: &FrameworkState) {
    let chain = r#"
INPUT

RETURN out=$(MultiplyNode($(AddNode(1.0, 2.0)), 2.0))
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
    println!("✅ [test_chained_pure]  out = {}", out);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3: 工作流入参（使用 input.pin 语法）
// ─────────────────────────────────────────────────────────────────────────────

/// INPUT x:Any, y:Any — 显式声明入参引脚
/// 传入 x=15.0, y=27.0 → 期望 result = 42.0
async fn test_input_params(framework: &FrameworkState) {
    let chain = r#"
INPUT x:Any y:Any

RETURN result=$(AddNode(input.x, input.y))
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
    println!("✅ [test_input_params]  result = {}", result);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4: FOR 1 TO 10 + Pure 节点（"1加到10"核心测试）
// ─────────────────────────────────────────────────────────────────────────────

/// $num = 0.0                                        — 初始化可变变量
/// FOR 1 TO 10:
///     SetVar($num, $(MultiplyNode($index, 1.0)))    — Impure 作循环体，$index 是隐式循环变量
/// RETURN last=$num                                  — 循环后读终值
///
/// 每次迭代：$num = $index × 1.0；最后一次 $index=10 → $num=10.0
async fn test_for_loop(framework: &FrameworkState) {
    let chain = r#"
INPUT
$num = 0.0
FOR 1 TO 10:
    SetVar($num, $(MultiplyNode($index, 1.0)))
RETURN last=$num
"#;

    let outputs = run_chain(chain, HashMap::new(), framework)
        .await
        .unwrap_or_else(|e| panic!("❌ [test_for_loop] 执行失败: {}", e));

    let last = outputs
        .get("last")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("❌ [test_for_loop] 缺少 'last' 输出，实际: {:?}", outputs));

    assert_eq!(
        last, 10.0,
        "❌ [test_for_loop] 期望最后一次迭代结果 10.0，实际 {}",
        last
    );
    println!("✅ [test_for_loop]  last index result = {}", last);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 5: FOR 1 TO 10 + IF/BREAK（提前中断循环）
// ─────────────────────────────────────────────────────────────────────────────

/// INPUT stop:Any=7（默认 7）
/// $num = 0.0
/// FOR 1 TO 10:
///     IF $(EqualNode(input.stop, $index)):
///         BREAK                              ← index == stop 时中断
///     ELSE:
///         SetVar($num, $(MultiplyNode($index, 1.0)))
/// RETURN last=$num
///
/// stop=7(默认)：最后一次 SetVar 是 index=6 → last=6.0
/// stop=4     ：最后一次 SetVar 是 index=3 → last=3.0
async fn test_for_loop_break(framework: &FrameworkState) {
    let chain = r#"
INPUT stop:Any=7
$num = 0.0
FOR 1 TO 10:
    IF $(EqualNode(input.stop, $index)):
        BREAK
    ELSE:
        SetVar($num, $(MultiplyNode($index, 1.0)))
RETURN last=$num
"#;

    // ── 场景 1: 不传 stop，使用默认值 7 ────────────────────────────────────
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
        "❌ [test_for_loop_break/default] 期望 6.0（在 index=7 前 break），实际 {}",
        last
    );
    println!("✅ [test_for_loop_break] stop=7(default)  last = {}", last);

    // ── 场景 2: 传入 stop=4 ─────────────────────────────────────────────────
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
        "❌ [test_for_loop_break/stop=4] 期望 3.0（在 index=4 前 break），实际 {}",
        last
    );
    println!("✅ [test_for_loop_break] stop=4  last = {}", last);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 6: FOREACH + 字符串拼接（构建句子）
// ─────────────────────────────────────────────────────────────────────────────

/// INPUT strarray:Array（字符串数组）
/// $sentence = ""
/// FOR input.strarray:                              — ForEachNode，$item/$index 隐式
///     SetVar($sentence, $(StringAppend(A=$(StringAppend(A=$sentence, B=" ")), B=$item.Element)))
/// RETURN sentence=$(TrimNode(A=$sentence))
///
/// ["hello","world","rust"] → "hello world rust"
/// ["foo","bar"]            → "foo bar"
async fn test_foreach_string_join(framework: &FrameworkState) {
    let chain = r#"
INPUT strarray:Array
$sentence = ""
FOR input.strarray:
    SetVar($sentence, $(StringAppendNode($(StringAppendNode($sentence, " ")), $item.Element)))
RETURN sentence=$(TrimNode($sentence))
"#;

    // ── 场景 1: ["hello", "world", "rust"] → "hello world rust" ────────────
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
        "❌ [test_foreach_string_join/3words] 期望 \"hello world rust\"，实际 \"{}\"",
        sentence
    );
    println!(
        "✅ [test_foreach_string_join] 3 words  sentence = \"{}\"",
        sentence
    );

    // ── 场景 2: ["foo", "bar"] → "foo bar" ──────────────────────────────────
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
        "❌ [test_foreach_string_join/2words] 期望 \"foo bar\"，实际 \"{}\"",
        sentence
    );
    println!(
        "✅ [test_foreach_string_join] 2 words  sentence = \"{}\"",
        sentence
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 7: N: IF / N: ELSE 带步骤编号的条件分支
// ─────────────────────────────────────────────────────────────────────────────

/// INPUT x:Any
/// $result = 0.0
/// 1: IF $(EqualNode(input.x, 0.0)):
///     SetVar($result, 0.0)
/// 2: ELSE:
///     SetVar($result, input.x)
/// RETURN result=$result
/// 传入 x=5.0 → result=5.0；传入 x=0.0 → result=0.0
async fn test_numbered_if(framework: &FrameworkState) {
    let chain = r#"
INPUT x:Any
$result = 0.0

1: IF $(EqualNode(input.x, 0.0)):
    SetVar($result, 0.0)
2: ELSE:
    SetVar($result, input.x)
RETURN result=$result
"#;

    // 场景1: x=5.0 → 走 ELSE 分支 → result=5.0
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
    println!("✅ [test_numbered_if] x=5.0  result = {}", result);

    // 场景2: x=0.0 → 走 IF 分支 → result=0.0
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
    println!("✅ [test_numbered_if] x=0.0  result = {}", result);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 8: N: FOR 带步骤编号的循环
// ─────────────────────────────────────────────────────────────────────────────

/// $sum = 0.0
/// 1: FOR 1 TO 5:
///     SetVar($sum, $(AddNode($sum, $index)))
/// RETURN total=$sum
/// 期望: 1+2+3+4+5 = 15.0
async fn test_numbered_for(framework: &FrameworkState) {
    let chain = r#"
INPUT
$sum = 0.0
1: FOR 1 TO 5:
    SetVar($sum, $(AddNode($sum, $index)))
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
    println!("✅ [test_numbered_for]  total = {}", total);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 9: 编译器解析 N: IF / N: FOR（仅验证 compile_chain 不报错）
// ─────────────────────────────────────────────────────────────────────────────

async fn test_numbered_syntax_parse(_framework: &FrameworkState) {
    // 完整的带编号语法：IF/ELIF/ELSE/FOR 全部带步骤编号
    let chain = r#"
INPUT x:Any
$x = 0.0

1: IF $(EqualNode(input.x, 1.0)):
    SetVar($x, 1.0)
2: ELIF $(EqualNode(input.x, 2.0)):
    SetVar($x, 2.0)
3: ELSE:
    SetVar($x, 0.0)
4: FOR 1 TO 3:
    4.1: IF $(EqualNode($index, 2)):
        BREAK
RETURN
"#;

    compile_chain(chain)
        .unwrap_or_else(|e| panic!("❌ [test_numbered_syntax_parse] 解析失败: {}", e));
    println!("✅ [test_numbered_syntax_parse] N: IF/ELIF/ELSE/FOR 全部解析成功");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 10: 第2行空行，第3行节点调用（用户实际场景）
// ─────────────────────────────────────────────────────────────────────────────

/// INPUT (第1行)
/// (第2行空行)
/// 1: OpenBrowser(...) (第3行) → 应该是合法的
/// RETURN (第4行)
async fn test_second_line_empty(_framework: &FrameworkState) {
    let chain = r#"
INPUT

1: OpenBrowser(url="https://creator.douyin.com/")
RETURN
"#;

    // 解析应该成功（第2行是空行，第3行是节点调用是合法的）
    // 这是用户实际场景，第2行空行，第3行节点调用
    let _bp = compile_chain(chain)
        .map_err(|e| FrameworkError::WorkflowError(format!("compile_chain 失败: {}", e)))
        .unwrap();
    println!("✅ [test_second_line_empty] 解析成功！第2行空行，第3行节点调用");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 11: 第2行直接是节点调用（应该报错）
// ─────────────────────────────────────────────────────────────────────────────

async fn test_second_line_node_call_should_fail(_framework: &FrameworkState) {
    // 第2行直接是节点调用，没有变量声明，应该报错
    let chain = r#"
INPUT
OpenBrowser(url="https://creator.douyin.com/")
RETURN
"#;

    let result = compile_chain(chain);
    if let Err(err) = result {
        println!(
            "✅ [test_second_line_node_call_should_fail] 正确报错: {}",
            err
        );
    } else {
        panic!("❌ [test_second_line_node_call_should_fail] 应该报错但没有！");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 主函数
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    println!("\n╔══════════════════════════════════════════════════════╗");
    println!("║        chain 编译器 · 端到端执行测试                ║");
    println!("╚══════════════════════════════════════════════════════╝\n");

    // 框架全局初始化（OnceLock 保证只初始化一次，多次调用安全）
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
    test_second_line_empty(&framework).await;
    test_second_line_node_call_should_fail(&framework).await;

    println!("\n╔══════════════════════════════════════════════════════╗");
    println!("║  🎉  全部 11 项测试通过！通路验证成功               ║");
    println!("╚══════════════════════════════════════════════════════╝\n");
}
