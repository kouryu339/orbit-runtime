//! 测试 DashScope API 是否接受 role: "tool" 的消息
//!
//! 用法: DASHSCOPE_API_KEY=sk-xxx cargo run -p aliyun-apis --example test_tool_role

use reqwest::Client;
use serde_json::{json, Value};

#[tokio::main]
async fn main() {
    let api_key = std::env::var("DASHSCOPE_API_KEY").expect("请设置 DASHSCOPE_API_KEY 环境变量");
    let client = Client::new();

    // ========================================================================
    // 第 1 轮：发送带 tools 定义的请求，让模型触发 tool_call
    // ========================================================================
    println!("=== 第 1 轮：请求模型调用工具 ===");

    let tools = json!([{
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "查询指定城市的天气",
            "parameters": {
                "type": "object",
                "properties": {
                    "city": {
                        "type": "string",
                        "description": "城市名"
                    }
                },
                "required": ["city"]
            }
        }
    }]);

    let messages_round1 = json!([
        {"role": "user", "content": "北京今天天气怎么样？"}
    ]);

    let body1 = json!({
        "model": "qwen-plus",
        "input": { "messages": messages_round1 },
        "parameters": {
            "result_format": "message",
            "tools": tools
        }
    });

    let resp1: Value = client
        .post("https://dashscope.aliyuncs.com/api/v1/services/aigc/text-generation/generation")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body1)
        .send()
        .await
        .expect("请求失败")
        .json()
        .await
        .expect("解析失败");

    println!(
        "第 1 轮响应:\n{}\n",
        serde_json::to_string_pretty(&resp1).unwrap()
    );

    // 提取 assistant 的回复（含 tool_calls）
    let assistant_msg = &resp1["output"]["choices"][0]["message"];
    let tool_calls = &assistant_msg["tool_calls"];

    if tool_calls.is_null() || !tool_calls.is_array() {
        println!("模型没有触发 tool_call，直接回复了文本。测试结束。");
        println!("assistant content: {}", assistant_msg["content"]);
        return;
    }

    let tool_call_id = tool_calls[0]["id"].as_str().unwrap_or("unknown");
    let function_name = tool_calls[0]["function"]["name"]
        .as_str()
        .unwrap_or("unknown");
    let function_args = tool_calls[0]["function"]["arguments"]
        .as_str()
        .unwrap_or("{}");

    println!("模型请求调用工具: {} (id={})", function_name, tool_call_id);
    println!("参数: {}", function_args);

    // ========================================================================
    // 第 2 轮：提交 role="tool" 的消息，看 API 是否接受
    // ========================================================================
    println!("\n=== 第 2 轮：提交 tool 角色消息 ===");

    let tool_result = json!({"city": "北京", "weather": "晴", "temperature": "25°C"}).to_string();

    let messages_round2 = json!([
        {"role": "user", "content": "北京今天天气怎么样？"},
        assistant_msg,
        {
            "role": "tool",
            "content": tool_result,
            "tool_call_id": tool_call_id,
            "name": function_name
        }
    ]);

    let body2 = json!({
        "model": "qwen-plus",
        "input": { "messages": messages_round2 },
        "parameters": {
            "result_format": "message",
            "tools": tools
        }
    });

    println!(
        "发送请求体:\n{}\n",
        serde_json::to_string_pretty(&body2).unwrap()
    );

    let resp2: Value = client
        .post("https://dashscope.aliyuncs.com/api/v1/services/aigc/text-generation/generation")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body2)
        .send()
        .await
        .expect("请求失败")
        .json()
        .await
        .expect("解析失败");

    println!(
        "第 2 轮响应:\n{}",
        serde_json::to_string_pretty(&resp2).unwrap()
    );

    // 判断结果
    if let Some(code) = resp2.get("code").and_then(|c| c.as_str()) {
        if !code.is_empty() {
            println!("\n❌ DashScope 拒绝了 tool 角色！错误码: {}", code);
            println!(
                "错误信息: {}",
                resp2.get("message").and_then(|m| m.as_str()).unwrap_or("?")
            );
        } else {
            println!("\n✅ DashScope 接受了 tool 角色！");
        }
    } else {
        let content = resp2["output"]["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("(无内容)");
        println!("\n✅ DashScope 接受了 tool 角色！模型回复: {}", content);
    }
}
