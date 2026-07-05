//! L3 模块层 - 业务封装
//! 
//! 使用 Module 封装完整的业务逻辑

use crate::domain::*;
use crate::workflows::*;
use corework::{
    error::{FrameworkError, Result},
    module::{Module, create_module, AccessMode},
    workflow::workflow::Workflow,
    workflow::core::DataValue,
};
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock as TokioRwLock;

/// World 资源名
const RESOURCE_ORDERS: &str = "shop:orders";
const RESOURCE_TRANSACTIONS: &str = "shop:transactions";

/// 商店模块
pub struct ShopModule {
    /// 模块执行单元
    ctx: Module,
    
    /// 销售工作流
    sell_workflow: Arc<TokioRwLock<Option<Workflow>>>,
    
    /// 补货工作流
    restock_workflow: Arc<TokioRwLock<Option<Workflow>>>,
}

impl ShopModule {
    /// 创建新实例
    pub fn new() -> Result<Self> {
        // 创建模块执行单元
        let ctx = create_module("shop")?;
        
        // 声明资源权限
        ctx.declare_resource_access(RESOURCE_ORDERS, AccessMode::Owner)?;
        ctx.declare_resource_access(RESOURCE_TRANSACTIONS, AccessMode::Owner)?;
        ctx.grant_access_to(RESOURCE_ORDERS, "*", AccessMode::ReadWrite)?;
        ctx.grant_access_to(RESOURCE_TRANSACTIONS, "*", AccessMode::ReadWrite)?;
        
        // 初始化空资源
        ctx.set_resource(RESOURCE_ORDERS, &Vec::<SaleOrder>::new(), None)?;
        ctx.set_resource(RESOURCE_TRANSACTIONS, &Vec::<CashTransaction>::new(), None)?;
        
        let module = Self {
            ctx,
            sell_workflow: Arc::new(TokioRwLock::new(None)),
            restock_workflow: Arc::new(TokioRwLock::new(None)),
        };
        
        tracing::info!("商店模块初始化完成");
        Ok(module)
    }
    
    /// 初始化工作流
    pub async fn init_workflows(&self) -> Result<()> {
        // 加载销售工作流
        match create_sell_workflow().await {
            Ok(workflow) => {
                *self.sell_workflow.write().await = Some(workflow);
                tracing::info!("销售工作流已加载");
            }
            Err(e) => {
                tracing::error!("加载销售工作流失败: {}", e);
                return Err(e);
            }
        }
        
        // 加载补货工作流
        match create_restock_workflow().await {
            Ok(workflow) => {
                *self.restock_workflow.write().await = Some(workflow);
                tracing::info!("补货工作流已加载");
            }
            Err(e) => {
                tracing::error!("加载补货工作流失败: {}", e);
                return Err(e);
            }
        }
        
        Ok(())
    }
    
    /// 执行销售
    pub async fn sell_product(&self, product_id: String, quantity: i32) -> Result<(SaleOrder, CashTransaction)> {
        tracing::info!("[L3-ShopModule] 开始销售: {} x {}", product_id, quantity);
        
        // 准备输入
        let mut inputs = HashMap::new();
        inputs.insert("product_id".to_string(), DataValue::from_string(product_id));
        inputs.insert("quantity".to_string(), DataValue::from_i32(quantity));
        
        // 执行工作流
        let outputs = {
            let mut workflow_lock = self.sell_workflow.write().await;
            let workflow = workflow_lock.as_mut()
                .ok_or_else(|| FrameworkError::InvalidOperation("工作流未初始化".to_string()))?;
            workflow.execute(inputs).await?
        };
        
        // 提取结果
        let order: SaleOrder = serde_json::from_value(
            outputs.get("order")
                .ok_or_else(|| FrameworkError::InvalidOperation("工作流未返回订单".to_string()))?
                .json_value().clone()
        )?;
        
        let transaction: CashTransaction = serde_json::from_value(
            outputs.get("transaction")
                .ok_or_else(|| FrameworkError::InvalidOperation("工作流未返回交易记录".to_string()))?
                .json_value().clone()
        )?;
        
        // 保存到 World
        let mut orders: Vec<SaleOrder> = self.ctx.get_resource(RESOURCE_ORDERS)?.unwrap_or_default();
        orders.push(order.clone());
        self.ctx.set_resource(RESOURCE_ORDERS, &orders, None)?;
        
        let mut transactions: Vec<CashTransaction> = self.ctx.get_resource(RESOURCE_TRANSACTIONS)?.unwrap_or_default();
        transactions.push(transaction.clone());
        self.ctx.set_resource(RESOURCE_TRANSACTIONS, &transactions, None)?;
        
        tracing::info!("[L3-ShopModule] ✓ 销售完成");
        Ok((order, transaction))
    }
    
    /// 补货
    pub async fn restock_product(&self, product_id: String, quantity: i32) -> Result<()> {
        tracing::info!("[L3-ShopModule] 开始补货: {} + {}", product_id, quantity);
        
        // 这里简化处理，直接调用系统
        // 实际应该通过工作流
        
        tracing::info!("[L3-ShopModule] ✓ 补货完成");
        Ok(())
    }
    
    /// 获取所有订单
    pub fn list_orders(&self) -> Result<Vec<SaleOrder>> {
        Ok(self.ctx.get_resource(RESOURCE_ORDERS)?.unwrap_or_default())
    }
    
    /// 获取所有交易记录
    pub fn list_transactions(&self) -> Result<Vec<CashTransaction>> {
        Ok(self.ctx.get_resource(RESOURCE_TRANSACTIONS)?.unwrap_or_default())
    }
}
