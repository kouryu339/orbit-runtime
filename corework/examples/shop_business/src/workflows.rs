//! L2 工作流构建
//! 
//! 使用 BlueprintBuilder 构建业务工作流

use crate::domain::*;
use crate::nodes::*;
use corework::prelude::*;
use corework::workflow::builder::BlueprintBuilder;
use corework::workflow::nodes::control::PinMapping;
use corework::workflow::workflow::Workflow;

// ============================================================================
// 销售工作流
// ============================================================================

/// 创建销售工作流
/// 
/// 流程：检查库存 → 创建订单 → 扣减库存 → 处理支付
pub async fn create_sell_workflow() -> Result<Workflow> {
    // 创建节点实例
    let check_stock = CheckStockNode::new("CheckStock");
    let create_order = CreateOrderNode::new("CreateOrder");
    let deduct_stock = DeductStockNode::new("DeductStock");
    let process_payment = ProcessPaymentNode::new("ProcessPayment");
    
    // 使用 Builder 构建工作流
    let workflow = BlueprintBuilder::new("SellWorkflow")
        // 开始节点（定义工作流输入）
        .add_start_with_outputs(
            "Start",
            vec![
                PinMapping::new("product_id", "sell::product_id", "String"),
                PinMapping::new("quantity", "sell::quantity", "i32"),
            ],
        )
        // 结束节点（定义工作流输出）
        .add_end_with_inputs(
            "End",
            vec![
                PinMapping::new("transaction", "sell::transaction", "CashTransaction"),
                PinMapping::new("order", "sell::order", "SaleOrder"),
            ],
        )
        // 添加业务节点
        .add_impure_node("CheckStock", check_stock)
        .add_impure_node("CreateOrder", create_order)
        .add_impure_node("DeductStock", deduct_stock)
        .add_impure_node("ProcessPayment", process_payment)
        
        // 连接执行流和数据流
        // Start → CheckStock
        .connect("Start", "Out", "CheckStock", "In")
        .connect("Start", "product_id", "CheckStock", "product_id")
        .connect("Start", "quantity", "CheckStock", "required_quantity")
        
        // CheckStock → CreateOrder（库存充足）
        .connect("CheckStock", "Sufficient", "CreateOrder", "In")
        .connect("Start", "product_id", "CreateOrder", "product_id")
        .connect("Start", "quantity", "CreateOrder", "quantity")
        
        // CreateOrder → DeductStock
        .connect("CreateOrder", "Then", "DeductStock", "In")
        .connect("Start", "product_id", "DeductStock", "product_id")
        .connect("Start", "quantity", "DeductStock", "quantity")
        
        // DeductStock → ProcessPayment
        .connect("DeductStock", "Then", "ProcessPayment", "In")
        .connect("CreateOrder", "order", "ProcessPayment", "order")
        
        // ProcessPayment → End
        .connect("ProcessPayment", "Then", "End", "In")
        .connect("ProcessPayment", "transaction", "End", "transaction")
        .connect("CreateOrder", "order", "End", "order")
        
        // 编译并构建
        .build()
        .await?;
    
    Ok(workflow)
}

// ============================================================================
// 补货工作流
// ============================================================================

/// 创建补货工作流
/// 
/// 流程：增加库存
pub async fn create_restock_workflow() -> Result<Workflow> {
    let workflow = BlueprintBuilder::new("RestockWorkflow")
        .add_start_with_outputs(
            "Start",
            vec![
                PinMapping::new("product_id", "restock::product_id", "String"),
                PinMapping::new("quantity", "restock::quantity", "i32"),
            ],
        )
        .add_end_with_inputs(
            "End",
            vec![
                PinMapping::new("success", "restock::success", "bool"),
            ],
        )
        .build()
        .await?;
    
    Ok(workflow)
}
