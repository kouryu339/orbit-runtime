//! 测试 OpenInExplorer 系统
//!
//! 通过 Context.get_dynamic_system() 按名称获取系统，然后传入 JSON 参数执行。
//!
//! ```bash
//! cargo run -p ai-assistant --example test_open_explorer
//! ```

use corework::world::FrameworkState;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🗂️  测试 OpenInExplorer 系统\n");

    // 1. 初始化框架（自动注册所有 buns_system）
    let framework = FrameworkState::initialize().expect("FrameworkState 初始化失败");

    let ctx = framework.create_context();

    // 2. 通过 ctx 按名称获取 OpenInExplorer 动态执行器
    let system = match ctx.get_dynamic_system("OpenInExplorer") {
        Ok(system) => system,
        Err(_) => {
            println!("   ℹ 未找到 OpenInExplorer 系统。该 example 仅在宿主进程额外链接对应工具 crate 时才能实际执行。");
            return Ok(());
        }
    };

    // 3. 构造输入参数 — 打开当前项目根目录
    let target_path = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();

    println!("   目标路径: {}", target_path);

    let mut input = HashMap::new();
    input.insert("path".to_string(), serde_json::json!(target_path));

    // 4. 执行
    match system.execute_dynamic(input, &ctx).await {
        Ok(result) => {
            println!("   ✅ 执行成功: {}", serde_json::to_string_pretty(&result)?);
        }
        Err(e) => {
            println!("   ❌ 执行失败: {:?}", e);
        }
    }

    Ok(())
}
