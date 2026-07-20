//! 数据模型定义
//! 
//! 使用 #[buns_model] 和 #[buns_enum] 宏定义业务数据结构

use serde::{Deserialize, Serialize};
use corework::{buns_model, buns_enum};

// ============================================================================
// 商品模型
// ============================================================================

/// 商品信息
#[buns_model("Product", "1.0.0", "Product information", "Shop", exportable = true)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    /// 商品ID
    pub id: String,
    /// 商品名称
    pub name: String,
    /// 价格
    pub price: f64,
    /// 库存数量
    pub stock: i32,
}

impl Product {
    pub fn new(id: String, name: String, price: f64, stock: i32) -> Self {
        Self { id, name, price, stock }
    }
}

// ============================================================================
// 订单模型
// ============================================================================

/// 订单状态
#[buns_enum("OrderStatus", "Order status", "Shop", exportable = true)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    /// 待处理
    Pending,
    /// 已完成
    Completed,
    /// 已取消
    Cancelled,
    /// 失败
    Failed,
}

/// 销售订单
#[buns_model("SaleOrder", "1.0.0", "Sale order", "Shop", exportable = true)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaleOrder {
    /// 订单ID
    pub order_id: String,
    /// 商品ID
    pub product_id: String,
    /// 商品名称
    pub product_name: String,
    /// 购买数量
    pub quantity: i32,
    /// 单价
    pub unit_price: f64,
    /// 总金额
    pub total_amount: f64,
    /// 订单状态
    pub status: OrderStatus,
    /// 创建时间
    pub created_at: String,
}

// ============================================================================
// 补货模型
// ============================================================================

/// 补货记录
#[buns_model("RestockRecord", "1.0.0", "Restock record", "Shop", exportable = true)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestockRecord {
    /// 记录ID
    pub record_id: String,
    /// 商品ID
    pub product_id: String,
    /// 商品名称
    pub product_name: String,
    /// 补货数量
    pub quantity: i32,
    /// 补货成本
    pub cost: f64,
    /// 补货时间
    pub restocked_at: String,
}

// ============================================================================
// 交易模型
// ============================================================================

/// 收银交易记录
#[buns_model("CashTransaction", "1.0.0", "Cash transaction", "Shop", exportable = true)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashTransaction {
    /// 交易ID
    pub transaction_id: String,
    /// 订单ID
    pub order_id: String,
    /// 交易金额
    pub amount: f64,
    /// 交易时间
    pub timestamp: String,
}

// ============================================================================
// 系统输入输出模型
// ============================================================================

/// 库存查询输入
#[buns_model("QueryStockInput", "1.0.0", "Query stock input", "Shop", exportable = false)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryStockInput {
    pub product_id: String,
}

/// 扣减库存输入
#[buns_model("DeductStockInput", "1.0.0", "Deduct stock input", "Shop", exportable = false)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeductStockInput {
    pub product_id: String,
    pub quantity: i32,
}

/// 增加库存输入
#[buns_model("AddStockInput", "1.0.0", "Add stock input", "Shop", exportable = false)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddStockInput {
    pub product_id: String,
    pub quantity: i32,
}

/// 创建订单输入
#[buns_model("CreateOrderInput", "1.0.0", "Create order input", "Shop", exportable = false)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOrderInput {
    pub product_id: String,
    pub quantity: i32,
}

/// 处理支付输入
#[buns_model("ProcessPaymentInput", "1.0.0", "Process payment input", "Shop", exportable = false)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessPaymentInput {
    pub order_id: String,
    pub amount: f64,
}
