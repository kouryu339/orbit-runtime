//! 节点包装器 - 解决 trait object downcast 问题
//!
//! 使用枚举来统一不同类型的节点

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::Result;
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::control::{BranchNode, EndNode, PinMapping, StartNode};
use crate::workflow::nodes::data::logic::GreaterNode;
use crate::workflow::nodes::data::math::{AddNode, MultiplyNode};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};

/// 节点包装器 - 统一所有节点类型
#[derive(Clone)]
pub enum NodeWrapper {
    // Pure 节点
    Add(AddNode),
    Multiply(MultiplyNode),
    Greater(GreaterNode),

    // Impure 节点
    Branch(BranchNode),
    Start(StartNode),
    End(EndNode),

    /// 动态节点 - 支持任意实现了 BlueprintNode 的类型
    /// 这允许从 JSON 加载任意注册的节点类型（Pure/Impure/Event/Latent）
    DynamicBlueprint(Arc<dyn BlueprintNode + Send + Sync>),
}

// 手动实现 Debug，因为 trait object 不支持自动派生
impl std::fmt::Debug for NodeWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Add(_) => write!(f, "NodeWrapper::Add"),
            Self::Multiply(_) => write!(f, "NodeWrapper::Multiply"),
            Self::Greater(_) => write!(f, "NodeWrapper::Greater"),
            Self::Branch(_) => write!(f, "NodeWrapper::Branch"),
            Self::Start(_) => write!(f, "NodeWrapper::Start"),
            Self::End(_) => write!(f, "NodeWrapper::End"),
            Self::DynamicBlueprint(_) => write!(f, "NodeWrapper::DynamicBlueprint"),
        }
    }
}

impl NodeWrapper {
    /// 创建 Add 节点
    pub fn add(_name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self::Add(AddNode))
    }

    /// 创建 Multiply 节点
    pub fn multiply(_name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self::Multiply(MultiplyNode))
    }

    /// 创建 Greater 节点
    pub fn greater(_name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self::Greater(GreaterNode))
    }

    /// 创建 Branch 节点
    pub fn branch(_name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self::Branch(BranchNode))
    }

    /// 创建 Start 节点
    pub fn start(_name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self::Start(StartNode::new()))
    }

    /// 创建带输出映射的 Start 节点
    pub fn start_with_outputs(_name: impl Into<String>, outputs: Vec<PinMapping>) -> Arc<Self> {
        Arc::new(Self::Start(StartNode::with_outputs(outputs)))
    }

    /// 创建 End 节点
    pub fn end(_name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self::End(EndNode::new()))
    }

    /// 创建带自定义缓存键的 End 节点
    pub fn end_with_cache_key(
        _name: impl Into<String>,
        cache_key: impl Into<String>,
        result_pin: impl Into<String>,
        result_type: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self::End(EndNode::with_cache_key(
            cache_key,
            result_pin,
            result_type,
        )))
    }

    /// 创建带输入映射的 End 节点
    pub fn end_with_inputs(_name: impl Into<String>, inputs: Vec<PinMapping>) -> Arc<Self> {
        Arc::new(Self::End(EndNode::with_inputs(inputs)))
    }

    /// 从任意 BlueprintNode 创建动态节点
    ///
    /// 这允许业务代码添加自定义节点：
    /// ```rust
    /// let custom_node = MyCustomNode::new("Custom");
    /// let wrapper = NodeWrapper::from_blueprint(custom_node);
    /// ```
    pub fn from_blueprint<N: BlueprintNode + Send + Sync + 'static>(node: N) -> Arc<Self> {
        Arc::new(Self::DynamicBlueprint(Arc::new(node)))
    }

    /// 执行 Pure 节点
    pub fn evaluate_pure(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        match self {
            Self::Add(node) => node.evaluate(inputs),
            Self::Multiply(node) => node.evaluate(inputs),
            Self::Greater(node) => node.evaluate(inputs),
            _ => Err(crate::error::FrameworkError::SystemError(
                "Not a pure node".into(),
            )),
        }
    }

    /// 执行 Impure 节点
    pub async fn execute_impure(
        &self,
        ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        match self {
            Self::Branch(node) => node.execute(ctx, inputs).await,
            Self::Start(node) => node.execute(ctx, inputs).await,
            Self::End(node) => node.execute(ctx, inputs).await,
            Self::DynamicBlueprint(blueprint_node) => {
                // 调用统一的 execute_node 接口（由 register_node 宏生成）
                blueprint_node.execute_node(ctx, inputs).await
            }
            _ => Err(crate::error::FrameworkError::InvalidOperation(
                "Not an impure node".into(),
            )),
        }
    }
}

// 实现 BlueprintNode trait
impl BlueprintNode for NodeWrapper {
    fn name(&self) -> &str {
        match self {
            Self::Add(node) => node.name(),
            Self::Multiply(node) => node.name(),
            Self::Greater(node) => node.name(),
            Self::Branch(node) => node.name(),
            Self::Start(node) => node.name(),
            Self::End(node) => node.name(),
            Self::DynamicBlueprint(node) => node.name(),
        }
    }

    fn node_type(&self) -> NodeType {
        match self {
            Self::Add(node) => node.node_type(),
            Self::Multiply(node) => node.node_type(),
            Self::Greater(node) => node.node_type(),
            Self::Branch(node) => node.node_type(),
            Self::Start(node) => node.node_type(),
            Self::End(node) => node.node_type(),
            Self::DynamicBlueprint(node) => node.node_type(),
        }
    }

    fn pins(&self) -> Vec<Pin> {
        match self {
            Self::Add(node) => node.pins(),
            Self::Multiply(node) => node.pins(),
            Self::Greater(node) => node.pins(),
            Self::Branch(node) => node.pins(),
            Self::Start(node) => node.pins(),
            Self::End(node) => node.pins(),
            Self::DynamicBlueprint(node) => node.pins(),
        }
    }

    fn description(&self) -> Option<&str> {
        match self {
            Self::Add(node) => node.description(),
            Self::Multiply(node) => node.description(),
            Self::Greater(node) => node.description(),
            Self::Branch(node) => node.description(),
            Self::Start(node) => node.description(),
            Self::End(node) => node.description(),
            Self::DynamicBlueprint(node) => node.description(),
        }
    }

    fn category(&self) -> Option<&str> {
        match self {
            Self::Add(node) => node.category(),
            Self::Multiply(node) => node.category(),
            Self::Greater(node) => node.category(),
            Self::Branch(node) => node.category(),
            Self::Start(node) => node.category(),
            Self::End(node) => node.category(),
            Self::DynamicBlueprint(node) => node.category(),
        }
    }

    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut crate::workflow::execution::ExecutionContext,
        inputs: std::collections::HashMap<String, crate::workflow::core::DataValue>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = crate::error::Result<crate::workflow::core::NodeOutput>,
                > + Send
                + 'a,
        >,
    > {
        let variant = match self {
            Self::Add(_) => "Add",
            Self::Multiply(_) => "Multiply",
            Self::Greater(_) => "Greater",
            Self::Branch(_) => "Branch",
            Self::Start(_) => "Start",
            Self::End(_) => "End",
            Self::DynamicBlueprint(_) => "DynamicBlueprint",
        };
        tracing::debug!("      🔀 [NodeWrapper] 匹配到变体: {}", variant);

        match self {
            Self::Add(node) => node.execute_node(ctx, inputs),
            Self::Multiply(node) => node.execute_node(ctx, inputs),
            Self::Greater(node) => node.execute_node(ctx, inputs),
            Self::Branch(node) => node.execute_node(ctx, inputs),
            Self::Start(node) => node.execute_node(ctx, inputs),
            Self::End(node) => node.execute_node(ctx, inputs),
            Self::DynamicBlueprint(node) => {
                tracing::debug!(
                    "         🎭 [NodeWrapper] DynamicBlueprint 调用内部节点的 execute_node"
                );
                node.execute_node(ctx, inputs)
            }
        }
    }
}
