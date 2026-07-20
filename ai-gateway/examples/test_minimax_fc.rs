//! MiniMax FC 行为专项测试
//!
//! 针对 MiniMax M2.7 在不同请求参数组合下的 FC 响应行为做系统性测试：
//!
//!   1. 基础 FC：tool_choice=required，无 response_format
//!   2. FC + response_format=json_object
//!   3. FC + tool_choice 指定函数名
//!   4. FC + tool_choice 指定函数名 + response_format=json_object
//!   5. 无 FC（纯 JSON prompt），无 response_format
//!   6. 无 FC（纯 JSON prompt）+ response_format=json_object
//!
//! 每组测试发同一条消息，打印：
//!   - HTTP 状态 / API 错误
//!   - 是否返回 tool_call
//!   - arguments 是否合法 JSON
//!   - 原始 content（若有）
//!
//! 运行：
//!   cargo run --example test_minimax_fc -p llm-gateway

use reqwest::Client;
use serde_json::{json, Value};

const BASE_URL: &str = "https://api.minimaxi.com/v1";
const MODEL: &str = "MiniMax-M2.7";
const USER_MSG: &str = "请帮我想一个简短的自我介绍，我叫小明，是一名程序员。";

/// assistant_decide 工具定义（与生产代码一致）
fn decide_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "assistant_decide",
            "description": "AI 决策工具，用于表达当前决策",
            "parameters": {
                "type": "object",
                "properties": {
                    "to_state": {
                        "type": "string",
                        "enum": ["executing", "asking", "result"],
                        "description": "下一步目标状态"
                    },
                    "result": {
                        "type": "string",
                        "description": "to_state=result 时填写最终回复"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "to_state=asking 时填写向用户的提问"
                    },
                    "tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "to_state=executing 时要调用的工具列表"
                    }
                },
                "required": ["to_state"]
            }
        }
    })
}

fn base_messages() -> Value {
    json!([
        {
            "role": "system",
            "content": "你是一个 AI 助手，必须通过调用 assistant_decide 工具来表达每一个决策，禁止直接输出文字。"
        },
        {
            "role": "user",
            "content": USER_MSG
        }
    ])
}

struct TestCase {
    name: &'static str,
    use_fc: bool,
    tool_choice: Option<Value>,     // None = 不设置
    response_format: Option<Value>, // None = 不设置
}

/// 执行一次请求，返回原始响应 JSON
async fn do_request(client: &Client, api_key: &str, body: &Value) -> Result<Value, String> {
    let resp = client
        .post(format!("{}/chat/completions", BASE_URL))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| format!("HTTP 失败: {e}"))?;

    let status = resp.status();
    let val: Value = resp.json().await.map_err(|e| format!("解析失败: {e}"))?;

    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, val));
    }
    Ok(val)
}

/// 分析并打印结果
fn analyze(resp: &Value) {
    // API 层错误
    if let Some(err) = resp.get("error") {
        let code = err.get("code").and_then(|v| v.as_u64()).unwrap_or(0);
        let msg = err.get("message").and_then(|v| v.as_str()).unwrap_or("?");
        println!("  ❌ API error  code={} msg={}", code, msg);
        return;
    }

    let choice = &resp["choices"][0];
    let finish = choice["finish_reason"].as_str().unwrap_or("?");
    let message = &choice["message"];
    let content = message["content"].as_str().unwrap_or("").trim().to_string();

    // 检查 tool_calls
    let tc_arr = message["tool_calls"].as_array();
    if let Some(tcs) = tc_arr {
        if !tcs.is_empty() {
            let tc = &tcs[0];
            let name = tc["function"]["name"].as_str().unwrap_or("?");
            let args = tc["function"]["arguments"].as_str().unwrap_or("");
            let args_valid = serde_json::from_str::<Value>(args).is_ok();
            println!(
                "  ✅ tool_call  name={}  finish={}  args_valid={}",
                name, finish, args_valid
            );
            if args_valid {
                // 打印解析后的 arguments 摘要
                if let Ok(v) = serde_json::from_str::<Value>(args) {
                    let to_state = v["to_state"].as_str().unwrap_or("?");
                    let preview = v["result"]
                        .as_str()
                        .or_else(|| v["prompt"].as_str())
                        .unwrap_or("")
                        .chars()
                        .take(60)
                        .collect::<String>();
                    println!("     to_state={}  value={:?}", to_state, preview);
                }
            } else {
                // arguments 非 JSON，打印原始内容
                println!(
                    "  ⚠️  arguments 非 JSON: {:?}",
                    &args[..args.len().min(100)]
                );
            }
            return;
        }
    }

    // 没有 tool_call
    if content.is_empty() {
        println!("  ⚠️  空响应  finish={}", finish);
    } else {
        let preview = content.chars().take(80).collect::<String>();
        println!("  ⚠️  无 tool_call，直接输出文字  finish={}", finish);
        println!("     content: {:?}", preview);
    }
}

#[tokio::main]
async fn main() {
    // 从 AppData 加载配置，拿到 MiniMax API key
    if let Err(e) = llm_gateway::init_keys() {
        eprintln!("init_keys 失败: {e}");
        return;
    }
    let config = match llm_gateway::load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("加载配置失败: {e}");
            return;
        }
    };

    // 找 MiniMax provider 的 api_key
    let api_key = config
        .providers
        .iter()
        .find(|p| p.name.to_lowercase().contains("minimax") || p.api_key.starts_with("sk-cp"))
        .map(|p| p.api_key.clone())
        .unwrap_or_else(|| {
            eprintln!("未找到 MiniMax provider，请先在应用设置中配置");
            std::process::exit(1);
        });

    println!("MiniMax API key: {}...", &api_key[..api_key.len().min(16)]);
    println!("Model: {}\n{}\n", MODEL, "=".repeat(60));

    let client = Client::new();

    let cases = vec![
        TestCase {
            name: "①  FC only (tool_choice=required)",
            use_fc: true,
            tool_choice: Some(json!("required")),
            response_format: None,
        },
        TestCase {
            name: "②  FC + response_format=json_object",
            use_fc: true,
            tool_choice: Some(json!("required")),
            response_format: Some(json!({"type": "json_object"})),
        },
        TestCase {
            name: "③  FC + tool_choice={name}",
            use_fc: true,
            tool_choice: Some(
                json!({"type": "function", "function": {"name": "assistant_decide"}}),
            ),
            response_format: None,
        },
        TestCase {
            name: "④  FC + tool_choice={name} + response_format=json_object",
            use_fc: true,
            tool_choice: Some(
                json!({"type": "function", "function": {"name": "assistant_decide"}}),
            ),
            response_format: Some(json!({"type": "json_object"})),
        },
        TestCase {
            name: "⑤  无 FC（纯 JSON prompt）",
            use_fc: false,
            tool_choice: None,
            response_format: None,
        },
        TestCase {
            name: "⑥  无 FC + response_format=json_object",
            use_fc: false,
            tool_choice: None,
            response_format: Some(json!({"type": "json_object"})),
        },
    ];

    for case in &cases {
        println!("【{}】", case.name);

        let mut body = json!({
            "model": MODEL,
            "messages": base_messages(),
            "stream": false,
        });

        if case.use_fc {
            body["tools"] = json!([decide_tool()]);
            if let Some(ref tc) = case.tool_choice {
                body["tool_choice"] = tc.clone();
            }
        }
        if let Some(ref rf) = case.response_format {
            body["response_format"] = rf.clone();
        }

        match do_request(&client, &api_key, &body).await {
            Ok(resp) => analyze(&resp),
            Err(e) => println!("  ❌ 请求失败: {}", e),
        }

        // 避免触发限流
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        println!();
    }

    // 额外测试：FC + 多轮对话（包含 tool 历史）时的稳定性
    println!("【⑦  FC + 多轮历史（含 tool 消息）】");
    let multi_messages = json!([
        {
            "role": "system",
            "content": "你是 AI 助手，必须通过 assistant_decide 工具表达决策。"
        },
        { "role": "user", "content": "帮我查一下北京的天气" },
        {
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_abc123",
                "type": "function",
                "function": {
                    "name": "assistant_decide",
                    "arguments": "{\"to_state\":\"executing\",\"tools\":[\"GetWeather\"]}"
                }
            }]
        },
        {
            "role": "tool",
            "tool_call_id": "call_abc123",
            "content": "{\"weather\":\"晴\",\"temperature\":\"22°C\"}"
        },
        { "role": "user", "content": "好，现在把结果告诉我" }
    ]);

    let body7 = json!({
        "model": MODEL,
        "messages": multi_messages,
        "tools": [decide_tool()],
        "tool_choice": "required",
        "response_format": {"type": "json_object"},
        "stream": false,
    });

    match do_request(&client, &api_key, &body7).await {
        Ok(resp) => analyze(&resp),
        Err(e) => println!("  ❌ 请求失败: {}", e),
    }
    println!();

    println!("{}", "=".repeat(60));
    println!("测试完成");
    println!();
    println!("说明：");
    println!("  ✅ tool_call args_valid=true  → FC 正常，arguments 合法 JSON");
    println!("  ⚠️  无 tool_call              → 模型忽略了 FC，直接回复文字");
    println!("  ⚠️  args_valid=false          → 返回了 tool_call 但 arguments 非 JSON");
    println!("  ❌ API error / 请求失败        → 参数组合被 MiniMax 拒绝");
}
