//! 节点 Trait 定义
//!
//! **重构为对齐UE Blueprint设计：**
//! - 单一BlueprintNode trait（类似UE的UEdGraphNode）
//! - 通过NodeType enum区分Pure/Impure/Event等
//! - 移除PureNode/ImpureNode等多余trait分支

use crate::error::Result;
use crate::workflow::core::{DataValue, NodeOutput, Pin, PinCacheMapping, PinDirection, PinType};
use crate::workflow::execution::ExecutionContext;
use std::collections::HashMap;

/// 节点类型分类（对齐UE）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NodeType {
    /// 纯函数节点 - 无副作用，仅数据转换
    /// 特征：无exec引脚，输出完全由输入决定
    Pure,

    /// 非纯节点 - 有副作用，改变状态
    /// 特征：有exec引脚，可能修改状态、执行I/O
    Impure,

    /// 事件节点 - 蓝图的入口点
    /// 特征：无exec输入，由事件触发
    Event,

    /// 延迟节点 - 异步操作
    /// 特征：执行会暂停等待
    Latent,

    Macro,
}

/// BlueprintNode - 唯一的节点基础trait（对齐UE的UEdGraphNode）
///
/// **设计原则：**
/// 4. name()返回节点类型的静态名称（如"Add"），不是实例ID
///
/// **对比UE：**
/// ```cpp
/// class UEdGraphNode : public UObject {
///     FString GetNodeTitle();  // 静态类型名称
///     TArray<UEdGraphPin*> Pins;
/// };
/// ```
pub trait BlueprintNode: Send + Sync {
    /// 节点类型名称（静态常量，用于UI显示）
    ///
    /// 返回节点类型的固定名称，如"Add", "Branch", "Open Browser"
    /// 这不是实例ID，所有相同类型的节点返回相同的名称
    ///
    /// 对应UE的GetNodeTitle()
    fn name(&self) -> &str;

    /// 节点类型分类
    ///
    /// 返回Pure/Impure/Event等，用于executor决定执行策略
    /// 对应UE中通过引脚判断节点类型的逻辑
    fn node_type(&self) -> NodeType;

    /// 节点的引脚定义
    ///
    /// 返回所有输入/输出引脚的定义
    /// 对应UE的Pins数组
    fn pins(&self) -> Vec<Pin>;

    /// Pin到Cache的映射配置
    ///
    /// 返回输入pin和输出pin的cache key映射
    /// executor会使用这些映射来读写cache
    ///
    /// 默认实现：自动根据节点name和pins生成 "{name}:{pin_name}" 格式的映射
    fn pin_cache_mappings(&self) -> (Vec<PinCacheMapping>, Vec<PinCacheMapping>) {
        let node_name = self.name();
        let pins = self.pins();

        let mut inputs = Vec::new();
        let mut outputs = Vec::new();

        for pin in pins {
            // 只处理数据引脚，提取类型名
            if let PinType::Data(type_name) = &pin.pin_type {
                let mapping = PinCacheMapping::new(
                    &pin.name,
                    format!("{}:{}", node_name, pin.name),
                    type_name,
                );

                match pin.direction {
                    PinDirection::Input => inputs.push(mapping),
                    PinDirection::Output => outputs.push(mapping),
                }
            }
        }

        (inputs, outputs)
    }

    /// 节点描述（用于文档/UI工具提示）
    fn description(&self) -> Option<&str> {
        None
    }

    /// 节点分类（用于节点面板组织）
    ///
    /// 如"Math", "Control Flow", "Browser"等
    /// 对应UE的Category
    fn category(&self) -> Option<&str> {
        None
    }

    ///
    /// **所有节点**都通过这个方法执行，executor会根据node_type()决定策略：
    /// - Event节点：事件触发时执行
    ///
    /// **参数：**
    /// - ctx: 执行上下文（包含cache、event_bus等）
    /// - inputs: 输入引脚的数据
    ///
    /// **返回：**
    /// - Pure节点：返回NodeOutput::Data(outputs)
    /// - Impure节点：返回NodeOutput::ExecPin(next_pin)
    /// - Event节点：返回NodeOutput::ExecPin(output_pin)
    ///
    /// **默认实现：**
    /// 返回错误，提示使用register_node宏或手动实现
    #[allow(unused_variables)]
    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<NodeOutput>> + Send + 'a>> {
        Box::pin(async move {
            Err(crate::error::FrameworkError::InvalidOperation(format!(
                "Node '{}' (type: {:?}) does not implement execute_node. \
                        Use #[register_node] macro or implement manually.",
                self.name(),
                self.node_type()
            )))
        })
    }
}
