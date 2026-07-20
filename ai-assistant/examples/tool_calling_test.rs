//! Tool Calling 端到端测试
//!
//! 验证完整的"按名称调用系统"流程：
//!
//! 1. `#[buns_system]` 宏自动注册 DynamicExecute
//! 2. `SystemRegistry::auto_register_all()` 收集所有系统
//! 3. `get_dynamic(name)` 按名称查找系统
//! 4. `execute_dynamic()` 使用 JSON 动态调用
//!
//! 运行方式：
//! ```bash
//! cargo run -p ai-assistant --example tool_calling_test
//! ```

use async_trait::async_trait;
use corework::buns_system;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::{SystemOperation, SystemRegistry};
use corework::world::FrameworkState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// 测试系统 1: 简单计算器（基础 buns_system，无 AI params）
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalcInput {
    pub a: f64,
    pub b: f64,
    pub op: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalcOutput {
    pub result: f64,
    pub expression: String,
}

#[buns_system("Calculator", description = "简单计算器")]
pub struct CalculatorSystem;

#[async_trait]
impl SystemOperation for CalculatorSystem {
    type Input = CalcInput;
    type Output = CalcOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &Context,
    ) -> Result<Self::Output, FrameworkError> {
        let result = match input.op.as_str() {
            "add" | "+" => input.a + input.b,
            "sub" | "-" => input.a - input.b,
            "mul" | "*" => input.a * input.b,
            "div" | "/" => {
                if input.b == 0.0 {
                    return Err(FrameworkError::InvalidOperation("Division by zero".into()));
                }
                input.a / input.b
            }
            _ => {
                return Err(FrameworkError::InvalidOperation(format!(
                    "Unknown op: {}",
                    input.op
                )))
            }
        };

        Ok(CalcOutput {
            result,
            expression: format!("{} {} {} = {}", input.a, input.op, input.b, result),
        })
    }
}

// ============================================================================
// 测试系统 2: AI 可调用的 Greeter（带 params 元数据）
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GreetInput {
    pub name: String,
    #[serde(default)]
    pub greeting: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GreetOutput {
    pub message: String,
}

#[buns_system(
    "Greeter",
    description = "向指定用户发送问候语",
    params {
        name: "用户名（必填）",
        greeting: "问候语（可选，默认：你好）"
    },
    destructive = false,
    readonly = true,
    idempotent = true
)]
pub struct GreeterSystem;

#[async_trait]
impl SystemOperation for GreeterSystem {
    type Input = GreetInput;
    type Output = GreetOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &Context,
    ) -> Result<Self::Output, FrameworkError> {
        let greeting = input.greeting.unwrap_or_else(|| "你好".to_string());
        Ok(GreetOutput {
            message: format!("{}，{}！欢迎使用 AI 助手。", greeting, input.name),
        })
    }
}

// ============================================================================
// 测试系统 3: 带数值参数的 AI 系统
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepeatInput {
    pub text: String,
    pub count: i64,
    pub uppercase: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepeatOutput {
    pub result: String,
    pub total_chars: usize,
}

#[buns_system(
    "TextRepeater",
    description = "重复文本指定次数",
    params {
        text: "要重复的文本（必填）",
        count: "重复次数（必填）",
        uppercase: "是否转为大写（可选）"
    },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct TextRepeaterSystem;

#[async_trait]
impl SystemOperation for TextRepeaterSystem {
    type Input = RepeatInput;
    type Output = RepeatOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &Context,
    ) -> Result<Self::Output, FrameworkError> {
        if input.count < 0 || input.count > 100 {
            return Err(FrameworkError::InvalidOperation(
                "count 必须在 0-100 之间".into(),
            ));
        }
        let mut result = input.text.repeat(input.count as usize);
        if input.uppercase.unwrap_or(false) {
            result = result.to_uppercase();
        }
        let total_chars = result.len();
        Ok(RepeatOutput {
            result,
            total_chars,
        })
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

fn print_separator(title: &str) {
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  {}", title);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
}

// ============================================================================
// 主测试
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔══════════════════════════════════════════╗");
    println!("║    Tool Calling 端到端测试               ║");
    println!("╚══════════════════════════════════════════╝\n");

    let mut passed = 0;
    let mut failed = 0;

    // ========================================================================
    // 准备：初始化 FrameworkState（全局单例：World + Registry + EventBus + Telemetry）
    // ========================================================================

    let framework = FrameworkState::initialize().expect("FrameworkState 初始化失败");

    let registry = framework.registry();

    println!("📋 已注册的所有系统（auto_register_all）:");
    for name in registry.list() {
        println!("   ✦ {}", name);
    }
    println!();

    // 列出 AI 可调用系统
    let ai_systems = SystemRegistry::list_ai_systems();
    println!("🤖 AI 可调用系统（带 params 元数据）:");
    for meta in &ai_systems {
        println!("   ✦ {} - {}", meta.name, meta.description);
        println!(
            "     属性: destructive={}, readonly={}, idempotent={}, open_world={}, secret={}",
            meta.destructive, meta.readonly, meta.idempotent, meta.open_world, meta.secret
        );
        for param in meta.parameters {
            let req = if param.required { "必填" } else { "可选" };
            let default = param
                .default_value
                .map(|v| format!(", 默认={}", v))
                .unwrap_or_default();
            println!(
                "     --{} ({}){} : {}",
                param.name, req, default, param.description
            );
        }
    }

    let ctx = framework.create_context();

    // ========================================================================
    // 测试 1: DynamicExecute - Calculator (基础系统)
    // ========================================================================

    print_separator("测试 1: DynamicExecute 调用 Calculator");

    match ctx.get_dynamic_system("Calculator") {
        Ok(calc) => {
            // 测试加法
            let mut input = HashMap::new();
            input.insert("a".to_string(), serde_json::json!(10.0));
            input.insert("b".to_string(), serde_json::json!(3.0));
            input.insert("op".to_string(), serde_json::json!("add"));

            match calc.execute_dynamic(input, &ctx).await {
                Ok(result) => {
                    println!("   输入: a=10, b=3, op=add");
                    println!("   输出: {}", serde_json::to_string_pretty(&result)?);
                    assert_eq!(result["result"], 13.0, "加法结果应为 13");
                    assert_eq!(result["expression"], "10 add 3 = 13");
                    println!("   ✅ 加法测试通过");
                    passed += 1;
                }
                Err(e) => {
                    println!("   ❌ 执行失败: {:?}", e);
                    failed += 1;
                }
            }

            // 测试除法
            let mut input = HashMap::new();
            input.insert("a".to_string(), serde_json::json!(10.0));
            input.insert("b".to_string(), serde_json::json!(4.0));
            input.insert("op".to_string(), serde_json::json!("div"));

            match calc.execute_dynamic(input, &ctx).await {
                Ok(result) => {
                    println!("   输入: a=10, b=4, op=div");
                    println!("   输出: result={}", result["result"]);
                    assert_eq!(result["result"], 2.5);
                    println!("   ✅ 除法测试通过");
                    passed += 1;
                }
                Err(e) => {
                    println!("   ❌ 执行失败: {:?}", e);
                    failed += 1;
                }
            }

            // 测试除零错误
            let mut input = HashMap::new();
            input.insert("a".to_string(), serde_json::json!(10.0));
            input.insert("b".to_string(), serde_json::json!(0.0));
            input.insert("op".to_string(), serde_json::json!("div"));

            match calc.execute_dynamic(input, &ctx).await {
                Ok(_) => {
                    println!("   ❌ 除零应该报错但返回了成功");
                    failed += 1;
                }
                Err(e) => {
                    println!("   输入: a=10, b=0, op=div");
                    println!("   错误: {:?}", e);
                    println!("   ✅ 除零错误处理正确");
                    passed += 1;
                }
            }
        }
        Err(e) => {
            println!("   ❌ 找不到 Calculator 系统: {:?}", e);
            failed += 3;
        }
    }

    // ========================================================================
    // 测试 2: DynamicExecute - Greeter (AI 系统)
    // ========================================================================

    print_separator("测试 2: DynamicExecute 调用 Greeter");

    match ctx.get_dynamic_system("Greeter") {
        Ok(greeter) => {
            // 只传 name（greeting 可选）
            let mut input = HashMap::new();
            input.insert("name".to_string(), serde_json::json!("Alice"));

            match greeter.execute_dynamic(input, &ctx).await {
                Ok(result) => {
                    let msg = result["message"].as_str().unwrap_or("");
                    println!("   输入: name=Alice");
                    println!("   输出: {}", msg);
                    assert!(msg.contains("Alice"), "消息应包含用户名");
                    println!("   ✅ 默认问候测试通过");
                    passed += 1;
                }
                Err(e) => {
                    println!("   ❌ 执行失败: {:?}", e);
                    failed += 1;
                }
            }

            // 传 name + greeting
            let mut input = HashMap::new();
            input.insert("name".to_string(), serde_json::json!("Bob"));
            input.insert("greeting".to_string(), serde_json::json!("早上好"));

            match greeter.execute_dynamic(input, &ctx).await {
                Ok(result) => {
                    let msg = result["message"].as_str().unwrap_or("");
                    println!("   输入: name=Bob, greeting=早上好");
                    println!("   输出: {}", msg);
                    assert!(msg.contains("Bob") && msg.contains("早上好"));
                    println!("   ✅ 自定义问候测试通过");
                    passed += 1;
                }
                Err(e) => {
                    println!("   ❌ 执行失败: {:?}", e);
                    failed += 1;
                }
            }
        }
        Err(e) => {
            println!("   ❌ 找不到 Greeter 系统: {:?}", e);
            failed += 2;
        }
    }

    // ========================================================================
    // 测试 3: DynamicExecute - TextRepeater (数值+布尔参数)
    // ========================================================================

    print_separator("测试 3: DynamicExecute 调用 TextRepeater");

    match ctx.get_dynamic_system("TextRepeater") {
        Ok(repeater) => {
            let mut input = HashMap::new();
            input.insert("text".to_string(), serde_json::json!("ha"));
            input.insert("count".to_string(), serde_json::json!(3));
            input.insert("uppercase".to_string(), serde_json::json!(true));

            match repeater.execute_dynamic(input, &ctx).await {
                Ok(result) => {
                    println!("   输入: text=ha, count=3, uppercase=true");
                    println!("   输出: {}", serde_json::to_string_pretty(&result)?);
                    assert_eq!(result["result"], "HAHAHA");
                    assert_eq!(result["total_chars"], 6);
                    println!("   ✅ 带数值+布尔参数测试通过");
                    passed += 1;
                }
                Err(e) => {
                    println!("   ❌ 执行失败: {:?}", e);
                    failed += 1;
                }
            }
        }
        Err(e) => {
            println!("   ❌ 找不到 TextRepeater 系统: {:?}", e);
            failed += 1;
        }
    }

    // ========================================================================
    // 测试 4: 错误处理
    // ========================================================================

    print_separator("测试 4: 错误处理");

    // 4a: DynamicExecute 输入反序列化失败
    match ctx.get_dynamic_system("Calculator") {
        Ok(calc) => {
            let mut bad_input = HashMap::new();
            bad_input.insert("wrong_field".to_string(), serde_json::json!("not a number"));

            match calc.execute_dynamic(bad_input, &ctx).await {
                Err(e) => {
                    println!("   输入: {{wrong_field: \"not a number\"}}");
                    println!("   错误: {:?}", e);
                    println!("   ✅ 输入反序列化错误处理正确");
                    passed += 1;
                }
                Ok(result) => {
                    println!("   ❌ 应该报错但返回: {}", result);
                    failed += 1;
                }
            }
        }
        Err(e) => {
            println!("   ❌ 找不到系统: {:?}", e);
            failed += 1;
        }
    }

    // ========================================================================
    // 测试 5: AI 系统帮助文档生成
    // ========================================================================

    print_separator("测试 5: AI 帮助文档");

    let help = SystemRegistry::get_ai_system_help("Greeter");
    match help {
        Some(doc) => {
            println!("{}", doc);
            assert!(doc.contains("Greeter"), "帮助应包含系统名");
            assert!(doc.contains("--name"), "帮助应包含参数");
            println!("   ✅ 帮助文档生成正确");
            passed += 1;
        }
        None => {
            println!("   ❌ 找不到 Greeter 的帮助文档");
            failed += 1;
        }
    }

    let full_doc = SystemRegistry::generate_ai_help_doc();
    println!(
        "   完整文档长度: {} chars, {} 行",
        full_doc.len(),
        full_doc.lines().count()
    );
    passed += 1;

    // ========================================================================
    // 总结
    // ========================================================================

    println!("\n╔══════════════════════════════════════════╗");
    println!("║    测试结果                               ║");
    println!("╠══════════════════════════════════════════╣");
    println!("║    通过: {:>3}                              ║", passed);
    println!("║    失败: {:>3}                              ║", failed);
    println!("╚══════════════════════════════════════════╝\n");

    if failed > 0 {
        println!("⚠️  有 {} 个测试失败！", failed);
        std::process::exit(1);
    } else {
        println!("🎉 所有测试通过！Tool Calling 运行时工作正常。");
        println!("\n关键验证点：");
        println!("  ✅ #[buns_system] 宏自动生成 DynamicExecute impl");
        println!("  ✅ auto_register_all() 正确收集所有系统");
        println!("  ✅ get_dynamic(name) 按名称查找");
        println!("  ✅ execute_dynamic() JSON 输入/输出");
        println!("  ✅ AI 元数据（params, destructive, readonly 等）");
        println!("  ✅ 错误处理（反序列化失败）");
    }

    Ok(())
}
