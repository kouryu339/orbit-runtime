//! L2 节点层 - 蓝图节点
//! 
//! 使用 #[register_node] 宏定义可视化编排的节点

use crate::domain::*;
use crate::systems::*;
use corework::prelude::*;
use corework::workflow::core::{DataValue, NodeOutput, Pin};
use corework::workflow::nodes::traits::{BlueprintNode, NodeType};
use corework::workflow::execution::ExecutionContext;
use corework::register_node;
use std::collections::HashMap;

// ============================================================================
// 库存节点
// ============================================================================

/// 检查库存节点
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Shop/Inventory",
    display_name = "Check Stock",
    description = "Check if product stock is sufficient",
    permissions = 0,
    exec_in = ["In"],
    exec_out = ["Sufficient", "Insufficient"],
    data_in = [
        "product_id:String@商品ID",
        "required_quantity:i32@需要数量"
    ],
    data_out = ["product:Product@商品信息"]
)]
pub struct CheckStockNode {
    name: String,
}

impl Default for CheckStockNode {
    fn default() -> Self {
        Self { name: "CheckStock".to_string() }
    }
}

impl CheckStockNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub async fn execute(
        &self,
        ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        // 提取输入
        let product_id = inputs.get("product_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| FrameworkError::SystemError("Missing product_id".into()))?
            .to_string();
        
        let required_quantity = inputs.get("required_quantity")
            .and_then(|v| v.as_i32())
            .ok_or_else(|| FrameworkError::SystemError("Missing required_quantity".into()))?;

        // 调用 L1 系统
        let legacy_ctx = ctx.inner();
        let system = legacy_ctx.system_by_type::<QueryStockSystem>()?;
        
        match system.execute(QueryStockInput { product_id: product_id.clone() }, legacy_ctx).await? {
            Some(product) => {
                if product.stock >= required_quantity {
                    tracing::info!("[L2-CheckStock] ✓ 库存充足: {}/{}", required_quantity, product.stock);
                    
                    let mut data = HashMap::new();
                    let product_value = serde_json::to_value(&product)?;
                    data.insert("product".to_string(), DataValue::new("Product", product_value));
                    
                    Ok(NodeOutput::ExecPin("Sufficient".to_string()))
                } else {
                    tracing::warn!("[L2-CheckStock] ❌ 库存不足: 需要 {}, 剩余 {}", required_quantity, product.stock);
                    Ok(NodeOutput::ExecPin("Insufficient".to_string()))
                }
            }
            None => {
                tracing::error!("[L2-CheckStock] 商品不存在: {}", product_id);
                Err(FrameworkError::NotFoundError(format!("商品不存在: {}", product_id)))
            }
        }
    }
}

impl BlueprintNode for CheckStockNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("product_id", "String"),
            Pin::data_in("required_quantity", "i32"),
            Pin::exec_out("Sufficient"),
            Pin::exec_out("Insufficient"),
            Pin::data_out("product", "Product"),
        ]
    }
}

/// 扣减库存节点
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Shop/Inventory",
    display_name = "Deduct Stock",
    description = "Deduct product stock",
    permissions = 0,
    exec_in = ["In"],
    exec_out = ["Then", "Error"],
    data_in = [
        "product_id:String@商品ID",
        "quantity:i32@扣减数量"
    ],
    data_out = ["updated_product:Product@更新后的商品"]
)]
pub struct DeductStockNode {
    name: String,
}

impl Default for DeductStockNode {
    fn default() -> Self {
        Self { name: "DeductStock".to_string() }
    }
}

impl DeductStockNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub async fn execute(
        &self,
        ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let product_id = inputs.get("product_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| FrameworkError::SystemError("Missing product_id".into()))?
            .to_string();
        
        let quantity = inputs.get("quantity")
            .and_then(|v| v.as_i32())
            .ok_or_else(|| FrameworkError::SystemError("Missing quantity".into()))?;

        let legacy_ctx = ctx.inner();
        let system = legacy_ctx.system_by_type::<DeductStockSystem>()?;
        
        match system.execute(DeductStockInput { product_id, quantity }, legacy_ctx).await {
            Ok(updated_product) => {
                let mut data = HashMap::new();
                let product_value = serde_json::to_value(&updated_product)?;
                data.insert("updated_product".to_string(), DataValue::new("Product", product_value));
                
                Ok(NodeOutput::Data(data))
            }
            Err(e) => {
                tracing::error!("[L2-DeductStock] 扣减失败: {}", e);
                Ok(NodeOutput::ExecPin("Error".to_string()))
            }
        }
    }
}

impl BlueprintNode for DeductStockNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("product_id", "String"),
            Pin::data_in("quantity", "i32"),
            Pin::exec_out("Then"),
            Pin::exec_out("Error"),
            Pin::data_out("updated_product", "Product"),
        ]
    }
}

// ============================================================================
// 订单节点
// ============================================================================

/// 创建订单节点
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Shop/Order",
    display_name = "Create Order",
    description = "Create a new sale order",
    permissions = 0,
    exec_in = ["In"],
    exec_out = ["Then"],
    data_in = [
        "product_id:String@商品ID",
        "quantity:i32@购买数量"
    ],
    data_out = ["order:SaleOrder@订单信息"]
)]
pub struct CreateOrderNode {
    name: String,
}

impl Default for CreateOrderNode {
    fn default() -> Self {
        Self { name: "CreateOrder".to_string() }
    }
}

impl CreateOrderNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub async fn execute(
        &self,
        ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let product_id = inputs.get("product_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| FrameworkError::SystemError("Missing product_id".into()))?
            .to_string();
        
        let quantity = inputs.get("quantity")
            .and_then(|v| v.as_i32())
            .ok_or_else(|| FrameworkError::SystemError("Missing quantity".into()))?;

        let legacy_ctx = ctx.inner();
        let system = legacy_ctx.system_by_type::<CreateOrderSystem>()?;
        
        let order = system.execute(CreateOrderInput { product_id, quantity }, legacy_ctx).await?;
        
        let mut data = HashMap::new();
        let order_value = serde_json::to_value(&order)?;
        data.insert("order".to_string(), DataValue::new("SaleOrder", order_value));
        
        Ok(NodeOutput::Data(data))
    }
}

impl BlueprintNode for CreateOrderNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("product_id", "String"),
            Pin::data_in("quantity", "i32"),
            Pin::exec_out("Then"),
            Pin::data_out("order", "SaleOrder"),
        ]
    }
}

// ============================================================================
// 支付节点
// ============================================================================

/// 处理支付节点
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Shop/Payment",
    display_name = "Process Payment",
    description = "Process payment for an order",
    permissions = 0,
    exec_in = ["In"],
    exec_out = ["Then"],
    data_in = [
        "order:SaleOrder@订单信息"
    ],
    data_out = ["transaction:CashTransaction@交易记录"]
)]
pub struct ProcessPaymentNode {
    name: String,
}

impl Default for ProcessPaymentNode {
    fn default() -> Self {
        Self { name: "ProcessPayment".to_string() }
    }
}

impl ProcessPaymentNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub async fn execute(
        &self,
        ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let order: SaleOrder = serde_json::from_value(
            inputs.get("order")
                .ok_or_else(|| FrameworkError::SystemError("Missing order".into()))?
                .json_value().clone()
        )?;

        let legacy_ctx = ctx.inner();
        let system = legacy_ctx.system_by_type::<ProcessPaymentSystem>()?;
        
        let transaction = system.execute(
            ProcessPaymentInput {
                order_id: order.order_id,
                amount: order.total_amount,
            },
            legacy_ctx
        ).await?;
        
        let mut data = HashMap::new();
        let transaction_value = serde_json::to_value(&transaction)?;
        data.insert("transaction".to_string(), DataValue::new("CashTransaction", transaction_value));
        
        Ok(NodeOutput::Data(data))
    }
}

impl BlueprintNode for ProcessPaymentNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("order", "SaleOrder"),
            Pin::exec_out("Then"),
            Pin::data_out("transaction", "CashTransaction"),
        ]
    }
}
