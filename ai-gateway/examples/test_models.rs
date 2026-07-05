//! 多模型连通性测试
//!
//! 从 %APPDATA%/sunwoo/llm_config.json 读取 API Key，依次向各模型发送一条消息，打印回复。
//!
//! 运行：
//!   cargo run --example test_models -p llm-gateway

use llm_gateway::{call_llm, ChatMessage};

#[tokio::main]
async fn main() {
    // 初始化 key_store 索引（从 llm_config.json 加载）
    if let Err(e) = llm_gateway::init_keys() {
        eprintln!("init_keys 失败: {}", e);
        return;
    }

    let config = match llm_gateway::load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("加载配置失败: {}", e);
            return;
        }
    };

    // 收集所有启用的模型 (uid, model_id, provider_name)
    let models: Vec<_> = config
        .providers
        .iter()
        .flat_map(|p| {
            p.enabled_models
                .iter()
                .map(move |m| (m.uid, m.model_id.clone(), p.name.clone()))
        })
        .collect();

    if models.is_empty() {
        eprintln!("llm_config.json 中没有配置 enabledModels");
        return;
    }

    let messages = vec![ChatMessage::user("用一句话介绍你自己，包括你的模型名称。")];

    println!("测试 {} 个模型\n{}", models.len(), "=".repeat(50));

    for (uid, model_id, provider_name) in &models {
        print!("[{} @ {} (uid={})] ... ", model_id, provider_name, uid);
        match call_llm(*uid, &messages, None, None, None).await {
            Ok(resp) => {
                let preview = resp.content.chars().take(80).collect::<String>();
                let tokens = resp
                    .tokens
                    .map(|t| format!(" ({}+{}tok)", t.input_tokens, t.output_tokens))
                    .unwrap_or_default();
                println!("OK{}\n    {}", tokens, preview);
            }
            Err(e) => println!("FAIL\n    {}", e),
        }
    }
}
