//! MiniMax 打招呼 + FC 行为测试
//!
//! 测试用户只说「你好」时，强制 tool_choice 下模型如何决策。
//! 对比：required vs {name} vs 加/不加 response_format
//!
//! 运行：
//!   cargo run --example test_minimax_hello -p llm-gateway

use reqwest::Client;
use serde_json::{json, Value};

const BASE_URL: &str = "https://api.minimaxi.com/v1";
const MODEL: &str = "MiniMax-M2.7";

fn decide_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "assistant_decide",
            "description": "表达 AI 决策。每次回复必须调用此函数，不能直接输出文字。",
            "parameters": {
                "type": "object",
                "properties": {
                    "to_state": {
                        "type": "string",
                        "enum": ["executing", "asking", "result"],
                        "description": "下一步目标状态"
                    },
                    "result": { "type": "string", "description": "to_state=result 时的最终回复" },
                    "prompt": { "type": "string", "description": "to_state=asking 时向用户的提问" },
                    "tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "to_state=executing 时的工具列表"
                    }
                },
                "required": ["to_state"]
            }
        }
    })
}

fn hello_messages() -> Value {
    json!([
        {
            "role": "system",
            "content": "你是 AI 助手 Lumi。每次回复必须通过调用 assistant_decide 工具来表达，禁止直接输出文字。"
        },
        {
            "role": "user",
            "content": "你好"
        }
    ])
}

async fn run_case(client: &Client, api_key: &str, label: &str, body: &Value) {
    println!("【{}】", label);
    let resp = client
        .post(format!("{}/chat/completions", BASE_URL))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            println!("  ❌ 网络失败: {e}\n");
            return;
        }
    };

    let status = resp.status();
    let val: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            println!("  ❌ 解析失败: {e}\n");
            return;
        }
    };

    if !status.is_success() {
        let code = val["error"]["http_code"].as_str().unwrap_or("?");
        let msg = val["error"]["message"].as_str().unwrap_or("?");
        println!("  ❌ HTTP {} / API {} — {}\n", status, code, msg);
        return;
    }

    if let Some(err) = val.get("error") {
        println!("  ❌ API error: {}\n", err);
        return;
    }

    let finish = val["choices"][0]["finish_reason"].as_str().unwrap_or("?");
    let message = &val["choices"][0]["message"];
    let content = message["content"].as_str().unwrap_or("").trim().to_string();
    let tcs = message["tool_calls"].as_array();

    if let Some(arr) = tcs {
        if !arr.is_empty() {
            let tc = &arr[0];
            let name = tc["function"]["name"].as_str().unwrap_or("?");
            let args = tc["function"]["arguments"].as_str().unwrap_or("");
            let ok = serde_json::from_str::<Value>(args).is_ok();
            println!(
                "  ✅ tool_call  name={}  finish={}  args_valid={}",
                name, finish, ok
            );
            if ok {
                let v = serde_json::from_str::<Value>(args).unwrap();
                let to_state = v["to_state"].as_str().unwrap_or("?");
                let preview = v["result"]
                    .as_str()
                    .or_else(|| v["prompt"].as_str())
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect::<String>();
                println!("     to_state={}  value={:?}", to_state, preview);
            } else {
                println!("     raw args: {:?}", &args[..args.len().min(120)]);
            }
            println!();
            return;
        }
    }

    if content.is_empty() {
        println!("  ⚠️  空响应  finish={}\n", finish);
    } else {
        let preview = content.chars().take(100).collect::<String>();
        println!("  ⚠️  无 tool_call，直接文字  finish={}", finish);
        println!("     {:?}\n", preview);
    }
}

#[tokio::main]
async fn main() {
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
    let api_key = config
        .providers
        .iter()
        .find(|p| p.api_key.starts_with("sk-cp"))
        .map(|p| p.api_key.clone())
        .unwrap_or_else(|| {
            eprintln!("未找到 MiniMax key");
            std::process::exit(1);
        });

    println!(
        "Model: {}  (用户消息: \"你好\")\n{}\n",
        MODEL,
        "=".repeat(60)
    );

    let client = Client::new();
    let delay = std::time::Duration::from_millis(4000);

    // ① required，无 response_format
    let body = json!({
        "model": MODEL, "messages": hello_messages(), "stream": false,
        "tools": [decide_tool()],
        "tool_choice": "required"
    });
    run_case(&client, &api_key, "① tool_choice=required", &body).await;
    tokio::time::sleep(delay).await;

    // ② required + json_object
    let body = json!({
        "model": MODEL, "messages": hello_messages(), "stream": false,
        "tools": [decide_tool()],
        "tool_choice": "required",
        "response_format": {"type": "json_object"}
    });
    run_case(
        &client,
        &api_key,
        "② required + response_format=json_object",
        &body,
    )
    .await;
    tokio::time::sleep(delay).await;

    // ③ {name}，无 response_format
    let body = json!({
        "model": MODEL, "messages": hello_messages(), "stream": false,
        "tools": [decide_tool()],
        "tool_choice": {"type": "function", "function": {"name": "assistant_decide"}}
    });
    run_case(&client, &api_key, "③ tool_choice={name}", &body).await;
    tokio::time::sleep(delay).await;

    // ④ {name} + json_object
    let body = json!({
        "model": MODEL, "messages": hello_messages(), "stream": false,
        "tools": [decide_tool()],
        "tool_choice": {"type": "function", "function": {"name": "assistant_decide"}},
        "response_format": {"type": "json_object"}
    });
    run_case(
        &client,
        &api_key,
        "④ tool_choice={name} + response_format=json_object",
        &body,
    )
    .await;
    tokio::time::sleep(delay).await;

    // ⑤ auto（不强制），有工具定义
    let body = json!({
        "model": MODEL, "messages": hello_messages(), "stream": false,
        "tools": [decide_tool()],
        "tool_choice": "auto"
    });
    run_case(&client, &api_key, "⑤ tool_choice=auto（不强制）", &body).await;

    println!("{}", "=".repeat(60));
}
