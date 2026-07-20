//! 豆包视觉 VLM 视频理解测试
//!
//! 用法：
//!   ARK_API_KEY=xxx cargo run -p llm-gateway --example test_vlm_video -- <视频路径> [提示词]
//!
//! 示例：
//!   ARK_API_KEY=xxx cargo run -p llm-gateway --example test_vlm_video -- ./test.mp4
//!   ARK_API_KEY=xxx cargo run -p llm-gateway --example test_vlm_video -- ./test.mp4 "视频里发生了什么？"

use llm_gateway::call_video_vlm;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let video_path = match args.get(1) {
        Some(p) => p.clone(),
        None => {
            eprintln!(
                "用法: cargo run -p llm-gateway --example test_vlm_video -- <视频路径> [提示词]"
            );
            eprintln!("示例: ARK_API_KEY=xxx cargo run -p llm-gateway --example test_vlm_video -- ./test.mp4");
            std::process::exit(1);
        }
    };

    let prompt = args
        .get(2)
        .map(|s| s.as_str())
        .unwrap_or("请详细描述这个视频的内容，包括画面中发生的事件、人物、场景等。");

    let model = "doubao-vision-pro-32k";

    // 检查文件是否存在
    if !std::path::Path::new(&video_path).exists() {
        eprintln!("❌ 视频文件不存在: {}", video_path);
        std::process::exit(1);
    }

    let file_size = std::fs::metadata(&video_path).map(|m| m.len()).unwrap_or(0);

    println!("═══════════════════════════════════════════════════");
    println!("  豆包 VLM 视频理解测试");
    println!("═══════════════════════════════════════════════════");
    println!("  模型  : {}", model);
    println!("  视频  : {}", video_path);
    println!("  大小  : {:.2} MB", file_size as f64 / 1024.0 / 1024.0);
    println!("  提示词: {}", prompt);
    println!("═══════════════════════════════════════════════════");
    println!("正在调用豆包视觉...\n");

    let start = std::time::Instant::now();

    match call_video_vlm(&video_path, prompt, Some(model), Some(2048)).await {
        Ok(resp) => {
            let elapsed = start.elapsed();
            println!("✅ 成功（耗时 {:.1}s）\n", elapsed.as_secs_f64());
            println!("─── 模型回复 ───────────────────────────────────");
            println!("{}", resp.content);
            println!("────────────────────────────────────────────────");
            if let Some(tokens) = resp.tokens {
                println!(
                    "\nToken 用量: 输入 {} / 输出 {}",
                    tokens.input_tokens, tokens.output_tokens
                );
            }
        }
        Err(e) => {
            let elapsed = start.elapsed();
            eprintln!("❌ 失败（耗时 {:.1}s）", elapsed.as_secs_f64());
            eprintln!("错误: {}", e);
            std::process::exit(1);
        }
    }
}
