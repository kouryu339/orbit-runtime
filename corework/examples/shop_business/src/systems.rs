//! L1 系统层 - 原子操作单元
//! 
//! 使用 #[buns_system] 宏定义无状态的系统操作

use crate::domain::*;
use corework::prelude::*;
use corework::buns_system;
use async_trait::async_trait;
use std::time::Duration;

// ============================================================================
// 库存系统
// ============================================================================

/// 查询商品库存
#[buns_system("QueryStockSystem")]
pub struct QueryStockSystem;

#[async_trait]
impl SystemOperation for QueryStockSystem {
    type Input = QueryStockInput;
    type Output = Option<Product>;
    type Error = FrameworkError;

    async fn execute(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        tracing::info!("  [L1-QueryStock] 查询商品库存: {}", input.product_id);
        
        let cache_key = format!("product:{}", input.product_id);
        let cache = ctx.get_cache();
        
        // 从缓存获取
        if let Some(product) = cache.get::<Product>(&cache_key).await? {
            tracing::info!("  [L1-QueryStock] ✓ 缓存命中");
            return Ok(Some(product));
        }
        
        // 模拟数据库查询
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        tracing::warn!("  [L1-QueryStock] 商品不存在");
        Ok(None)
    }
}

/// 扣减库存
#[buns_system("DeductStockSystem")]
pub struct DeductStockSystem;

#[async_trait]
impl SystemOperation for DeductStockSystem {
    type Input = DeductStockInput;
    type Output = Product;
    type Error = FrameworkError;

    async fn execute(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        tracing::info!("  [L1-DeductStock] 扣减库存: {} - {}", input.product_id, input.quantity);
        
        let cache_key = format!("product:{}", input.product_id);
        let cache = ctx.get_cache();
        
        let mut product = cache.get::<Product>(&cache_key).await?
            .ok_or_else(|| FrameworkError::NotFoundError(format!("商品不存在: {}", input.product_id)))?;
        
        if product.stock < input.quantity {
            return Err(FrameworkError::InvalidOperation(
                format!("库存不足: 需要 {}, 剩余 {}", input.quantity, product.stock)
            ));
        }
        
        product.stock -= input.quantity;
        cache.set(&cache_key, &product, Some(Duration::from_secs(3600))).await?;
        
        tracing::info!("  [L1-DeductStock] ✓ 扣减成功，剩余库存: {}", product.stock);
        Ok(product)
    }
}

/// 增加库存
#[buns_system("AddStockSystem")]
pub struct AddStockSystem;

#[async_trait]
impl SystemOperation for AddStockSystem {
    type Input = AddStockInput;
    type Output = Product;
    type Error = FrameworkError;

    async fn execute(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        tracing::info!("  [L1-AddStock] 增加库存: {} + {}", input.product_id, input.quantity);
        
        let cache_key = format!("product:{}", input.product_id);
        let cache = ctx.get_cache();
        
        let mut product = cache.get::<Product>(&cache_key).await?
            .ok_or_else(|| FrameworkError::NotFoundError(format!("商品不存在: {}", input.product_id)))?;
        
        product.stock += input.quantity;
        cache.set(&cache_key, &product, Some(Duration::from_secs(3600))).await?;
        
        tracing::info!("  [L1-AddStock] ✓ 补货成功，当前库存: {}", product.stock);
        Ok(product)
    }
}

/// 更新商品信息（用于初始化）
#[buns_system("UpdateProductSystem")]
pub struct UpdateProductSystem;

#[async_trait]
impl SystemOperation for UpdateProductSystem {
    type Input = Product;
    type Output = ();
    type Error = FrameworkError;

    async fn execute(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        tracing::info!("  [L1-UpdateProduct] 更新商品: {}", input.name);
        
        let cache_key = format!("product:{}", input.id);
        let cache = ctx.get_cache();
        cache.set(&cache_key, &input, Some(Duration::from_secs(3600))).await?;
        
        Ok(())
    }
}

// ============================================================================
// 订单系统
// ============================================================================

/// 创建订单
#[buns_system("CreateOrderSystem")]
pub struct CreateOrderSystem;

#[async_trait]
impl SystemOperation for CreateOrderSystem {
    type Input = CreateOrderInput;
    type Output = SaleOrder;
    type Error = FrameworkError;

    async fn execute(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        tracing::info!("  [L1-CreateOrder] 创建订单: {} x {}", input.product_id, input.quantity);
        
        // 查询商品信息
        let cache_key = format!("product:{}", input.product_id);
        let cache = ctx.get_cache();
        let product = cache.get::<Product>(&cache_key).await?
            .ok_or_else(|| FrameworkError::NotFoundError(format!("商品不存在: {}", input.product_id)))?;
        
        // 创建订单
        let order = SaleOrder {
            order_id: format!("ORD-{}", uuid::Uuid::new_v4()),
            product_id: product.id.clone(),
            product_name: product.name.clone(),
            quantity: input.quantity,
            unit_price: product.price,
            total_amount: product.price * input.quantity as f64,
            status: OrderStatus::Pending,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        
        // 保存到缓存
        cache.set(
            &format!("order:{}", order.order_id),
            &order,
            Some(Duration::from_secs(86400)),
        ).await?;
        
        tracing::info!("  [L1-CreateOrder] ✓ 订单已创建: {}", order.order_id);
        Ok(order)
    }
}

/// 更新订单状态
#[buns_system("UpdateOrderStatusSystem")]
pub struct UpdateOrderStatusSystem;

#[async_trait]
impl SystemOperation for UpdateOrderStatusSystem {
    type Input = (String, OrderStatus); // (order_id, new_status)
    type Output = SaleOrder;
    type Error = FrameworkError;

    async fn execute(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        let (order_id, new_status) = input;
        tracing::info!("  [L1-UpdateOrderStatus] 更新订单状态: {} → {:?}", order_id, new_status);
        
        let cache_key = format!("order:{}", order_id);
        let cache = ctx.get_cache();
        
        let mut order = cache.get::<SaleOrder>(&cache_key).await?
            .ok_or_else(|| FrameworkError::NotFoundError(format!("订单不存在: {}", order_id)))?;
        
        order.status = new_status;
        cache.set(&cache_key, &order, Some(Duration::from_secs(86400))).await?;
        
        tracing::info!("  [L1-UpdateOrderStatus] ✓ 状态已更新");
        Ok(order)
    }
}

// ============================================================================
// 支付系统
// ============================================================================

/// 处理支付
#[buns_system("ProcessPaymentSystem")]
pub struct ProcessPaymentSystem;

#[async_trait]
impl SystemOperation for ProcessPaymentSystem {
    type Input = ProcessPaymentInput;
    type Output = CashTransaction;
    type Error = FrameworkError;

    async fn execute(&self, input: Self::Input, ctx: &Context) -> Result<Self::Output> {
        tracing::info!("  [L1-ProcessPayment] 处理支付: ¥{:.2}", input.amount);
        
        // 模拟支付处理
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        let transaction = CashTransaction {
            transaction_id: format!("TXN-{}", uuid::Uuid::new_v4()),
            order_id: input.order_id,
            amount: input.amount,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        
        // 保存到缓存
        let cache = ctx.get_cache();
        cache.set(
            &format!("transaction:{}", transaction.transaction_id),
            &transaction,
            Some(Duration::from_secs(86400)),
        ).await?;
        
        tracing::info!("  [L1-ProcessPayment] ✓ 支付成功: {}", transaction.transaction_id);
        Ok(transaction)
    }
}
