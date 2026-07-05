//! Skill 提示词构建演示
//!
//! 演示如何使用 SkillManager 构建三层渐进式提示词

use ai_assistant::SkillManager;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Skill 提示词构建演示 ===\n");

    // ========================================================================
    // 第 1 步：初始化 SkillManager
    // ========================================================================

    let skills_dir = Path::new("crates/ai-assistant/skills");
    let registry_path = skills_dir.join("skills.json");

    println!("📂 从注册表加载: {:?}", registry_path);
    let mut manager = SkillManager::from_registry(&registry_path).await?;
    println!("✅ 发现 {} 个技能\n", manager.len());

    // ========================================================================
    // Tier 1: 技能目录（始终在上下文中）
    // ========================================================================

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📋 Tier 1: 技能目录（元数据，~100 词/技能）");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let catalog = manager.catalog_prompt();
    println!("{}", catalog);
    println!(
        "📊 Token 估算: {} 字符 ≈ {} tokens",
        catalog.len(),
        catalog.len() / 4 // 粗略估算
    );

    // ========================================================================
    // Tier 2: 触发技能的详细指导
    // ========================================================================

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎯 Tier 2: 触发 'rust-coding' 技能");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    // 加载完整 Skill（包含 instructions）
    let skill = manager.load("rust-coding").await?;
    println!("✅ 技能已加载: {}", skill.name());
    println!("   描述: {}...\n", &skill.description()[..80]);

    // 构建技能提示词
    let skill_prompt = manager.skill_prompt("rust-coding").unwrap();
    println!("{}", &skill_prompt[..500]); // 只显示前 500 字符
    println!("... (省略)\n");
    println!(
        "📊 Token 估算: {} 字符 ≈ {} tokens",
        skill_prompt.len(),
        skill_prompt.len() / 4
    );

    // ========================================================================
    // Tier 3: 按需加载 Reference（演示，需要实际 reference 文件）
    // ========================================================================

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📚 Tier 3: Reference 按需加载（可选）");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    // 列出可用的 reference 文件
    if let Ok(refs) = manager.list_references("rust-coding").await {
        if refs.is_empty() {
            println!("ℹ️  rust-coding 没有 reference 文件");
        } else {
            println!("可用 reference:");
            for ref_file in refs {
                println!("  - {}", ref_file);
            }
        }
    }

    // ========================================================================
    // 实际使用场景演示
    // ========================================================================

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("💬 实际使用场景：构建完整对话 Prompt");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    // 场景：用户请求编写 Rust 异步代码
    let user_query = "帮我写一个 Rust 异步函数，从 HTTP API 获取数据";

    println!("👤 用户: {}", user_query);
    println!("\n构建系统 Prompt:\n");

    // 组装完整 prompt
    let mut full_prompt = String::new();

    // 1. 系统角色说明
    full_prompt.push_str("你是一个 Rust 编程助手。\n\n");

    // 2. Tier 1: 所有技能目录（AI 判断是否需要触发）
    full_prompt.push_str(&catalog);
    full_prompt.push_str("\n\n");

    // 3. AI 判断：用户问题匹配 "rust-coding" 技能的 description
    //    → 触发加载
    full_prompt.push_str("用户查询匹配 'rust-coding' 技能，加载详细指导：\n\n");

    // 4. Tier 2: rust-coding 的完整 instructions
    full_prompt.push_str(&skill_prompt);
    full_prompt.push_str("\n\n");

    // 5. 用户问题
    full_prompt.push_str(&format!("用户问题：{}\n", user_query));

    println!("📝 完整 Prompt 长度: {} 字符", full_prompt.len());
    println!("📊 估算 Token: {} tokens", full_prompt.len() / 4);

    // 保存到文件查看
    std::fs::write("target/skill_prompt_example.txt", &full_prompt)?;
    println!("\n✅ 完整 prompt 已保存到: target/skill_prompt_example.txt");

    // ========================================================================
    // 多技能组合示例
    // ========================================================================

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🔄 多技能组合场景");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    // 同时触发多个技能
    let active_skills = ["rust-coding", "example-skill"];
    let combined_prompt = manager.active_skills_prompt(&active_skills);

    println!("激活技能: {:?}", active_skills);
    println!("组合 Prompt 长度: {} 字符", combined_prompt.len());
    println!("📊 估算 Token: {} tokens", combined_prompt.len() / 4);

    Ok(())
}
