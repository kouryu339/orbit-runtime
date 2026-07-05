//! 商店业务示例 - 主程序
//! 
//! 展示完整的三层架构实现：L1 系统 → L2 节点 → L3 模块

use shop_business::*;
use corework::prelude::*;
use corework::world::FrameworkState;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("\n🏪 商店业务系统启动");
    println!("═══════════════════════════════════════════════════════════\n");

    // ========================================================================
    // 初始化框架
    // ========================================================================
    
    let _framework = FrameworkState::initialize()?;
    tracing::info!("✓ 框架已初始化");
    
    // ========================================================================
    // 初始化商品数据
    // ========================================================================
    
    println!("📦 初始化商品数据");
    println!("───────────────────────────────────────────────────────────\n");
    
    // 获取框架上下文（简化版，实际应该通过 Module）
    // 这里为了演示，我们直接操作
    let products = vec![
        Product::new("PROD-001".to_string(), "iPhone 15".to_string(), 5999.0, 10),
        Product::new("PROD-002".to_string(), "iPad Pro".to_string(), 6999.0, 5),
        Product::new("PROD-003".to_string(), "MacBook Air".to_string(), 7999.0, 3),
    ];
    
    for product in &products {
        println!("  ✓ {} - ¥{:.2} - 库存: {}", product.name, product.price, product.stock);
    }
    
    println!("\n═══════════════════════════════════════════════════════════\n");
    
    // ========================================================================
    // 创建 L3 模块
    // ========================================================================
    
    println!("🏢 初始化商店模块");
    println!("───────────────────────────────────────────────────────────\n");
    
    let shop_module = ShopModule::new()?;
    shop_module.init_workflows().await?;
    
    println!("  ✓ 商店模块已就绪");
    println!("  ✓ 销售工作流已加载");
    println!("  ✓ 补货工作流已加载");
    
    println!("\n═══════════════════════════════════════════════════════════\n");
    
    // ========================================================================
    // 场景 1: 正常销售流程
    // ========================================================================
    
    println!("📝 场景 1: 正常销售");
    println!("───────────────────────────────────────────────────────────");
    
    match shop_module.sell_product("PROD-001".to_string(), 2).await {
        Ok((order, transaction)) => {
            println!("\n✅ 销售成功！");
            println!("  订单ID: {}", order.order_id);
            println!("  商品: {} x {}", order.product_name, order.quantity);
            println!("  总金额: ¥{:.2}", order.total_amount);
            println!("  交易ID: {}", transaction.transaction_id);
        }
        Err(e) => {
            println!("\n❌ 销售失败: {}", e);
        }
    }
    
    println!("\n═══════════════════════════════════════════════════════════\n");
    
    // ========================================================================
    // 场景 2: 库存不足的销售（失败场景）
    // ========================================================================
    
    println!("📝 场景 2: 库存不足");
    println!("───────────────────────────────────────────────────────────");
    
    match shop_module.sell_product("PROD-003".to_string(), 5).await {
        Ok((order, transaction)) => {
            println!("\n✅ 销售成功！");
            println!("  订单ID: {}", order.order_id);
            println!("  交易ID: {}", transaction.transaction_id);
        }
        Err(e) => {
            println!("\n❌ 销售失败: {}", e);
            println!("  原因: 库存只有 3 个，无法满足 5 个的需求");
        }
    }
    
    println!("\n═══════════════════════════════════════════════════════════\n");
    
    // ========================================================================
    // 场景 3: 补货流程
    // ========================================================================
    
    println!("📝 场景 3: 补货");
    println!("───────────────────────────────────────────────────────────");
    
    match shop_module.restock_product("PROD-003".to_string(), 10).await {
        Ok(_) => {
            println!("\n✅ 补货成功！");
            println!("  商品: MacBook Air");
            println!("  补货数量: 10");
            println!("  当前库存: 13");
        }
        Err(e) => {
            println!("\n❌ 补货失败: {}", e);
        }
    }
    
    println!("\n═══════════════════════════════════════════════════════════\n");
    
    // ========================================================================
    // 场景 4: 补货后再次销售
    // ========================================================================
    
    println!("📝 场景 4: 补货后再次销售");
    println!("───────────────────────────────────────────────────────────");
    
    match shop_module.sell_product("PROD-003".to_string(), 5).await {
        Ok((order, transaction)) => {
            println!("\n✅ 销售成功！");
            println!("  订单ID: {}", order.order_id);
            println!("  商品: {} x {}", order.product_name, order.quantity);
            println!("  总金额: ¥{:.2}", order.total_amount);
            println!("  交易ID: {}", transaction.transaction_id);
        }
        Err(e) => {
            println!("\n❌ 销售失败: {}", e);
        }
    }
    
    println!("\n═══════════════════════════════════════════════════════════\n");
    
    // ========================================================================
    // 统计信息
    // ========================================================================
    
    println!("📊 业务统计");
    println!("───────────────────────────────────────────────────────────\n");
    
    let orders = shop_module.list_orders()?;
    let transactions = shop_module.list_transactions()?;
    
    println!("  总订单数: {}", orders.len());
    println!("  总交易数: {}", transactions.len());
    
    let total_revenue: f64 = transactions.iter().map(|t| t.amount).sum();
    println!("  总收入: ¥{:.2}", total_revenue);
    
    println!("\n═══════════════════════════════════════════════════════════\n");
    
    // ========================================================================
    // 架构总结
    // ========================================================================
    
    println!("📊 三层架构总结\n");
    println!("数据层 (domain.rs):");
    println!("  └─ Product, SaleOrder, OrderStatus, CashTransaction");
    println!("     使用 #[buns_model] 和 #[buns_enum] 宏自动注册");
    println!();
    println!("L1 系统层 (systems.rs):");
    println!("  ├─ QueryStockSystem      → 查询库存");
    println!("  ├─ DeductStockSystem     → 扣减库存");
    println!("  ├─ AddStockSystem        → 增加库存");
    println!("  ├─ CreateOrderSystem     → 创建订单");
    println!("  └─ ProcessPaymentSystem  → 处理支付");
    println!("     使用 #[buns_system] 宏定义无状态系统");
    println!();
    println!("L2 节点层 (nodes.rs):");
    println!("  ├─ CheckStockNode        → 检查库存节点");
    println!("  ├─ DeductStockNode       → 扣减库存节点");
    println!("  ├─ CreateOrderNode       → 创建订单节点");
    println!("  └─ ProcessPaymentNode    → 处理支付节点");
    println!("     使用 #[register_node] 宏定义可视化节点");
    println!();
    println!("L2 工作流 (workflows.rs):");
    println!("  ├─ create_sell_workflow()    → 销售工作流");
    println!("  └─ create_restock_workflow() → 补货工作流");
    println!("     使用 BlueprintBuilder 构建工作流");
    println!();
    println!("L3 模块层 (module.rs):");
    println!("  └─ ShopModule");
    println!("     ├─ 资源管理 (订单、交易记录)");
    println!("     ├─ 工作流编排");
    println!("     └─ 业务接口封装");
    println!("     使用 create_module() 创建模块执行单元");
    println!();
    println!("═══════════════════════════════════════════════════════════\n");
    
    println!("✓ 示例运行完成\n");
    
    Ok(())
}
