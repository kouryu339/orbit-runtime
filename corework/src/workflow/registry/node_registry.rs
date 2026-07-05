//! 节点元数据注册表

use crate::workflow::nodes::traits::BlueprintNode;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 引脚方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinKind {
    ExecInput,
    ExecOutput,
    DataInput,
    DataOutput,
}

/// 静态引脚元数据（用于注册）
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PinMetadata {
    pub name: &'static str,        // 引脚名
    pub kind: PinKind,             // 引脚类型（输入/输出，执行/数据）
    pub data_type: &'static str,   // 数据类型（对于数据引脚）
    pub description: &'static str, // 简介
    /// DataInput 引脚的默认值（JSON 字符串），追加节点时自动写入草稿
    pub default_value: Option<&'static str>,
}

/// 节点元数据
#[derive(Debug, Clone, Copy)]
pub struct NodeMetadata {
    pub node_type: &'static str,
    pub version: &'static str, // 节点版本号（语义版本："1.0.0"）
    pub category: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub pins: &'static [PinMetadata],
    pub permissions: NodePermissions,
    pub wildcard_constraints: &'static [(&'static str, &'static [&'static str])],
}

impl NodeMetadata {
    pub const fn new(
        node_type: &'static str,
        version: &'static str,
        category: &'static str,
        display_name: &'static str,
        description: &'static str,
        pins: &'static [PinMetadata],
        permissions: NodePermissions,
    ) -> Self {
        Self {
            node_type,
            version,
            category,
            display_name,
            description,
            pins,
            permissions,
            wildcard_constraints: &[],
        }
    }

    pub const fn with_wildcard_constraints(
        mut self,
        wildcard_constraints: &'static [(&'static str, &'static [&'static str])],
    ) -> Self {
        self.wildcard_constraints = wildcard_constraints;
        self
    }
}

/// 节点分类元数据
///
/// 通过 `register_category!` 宏注册，控制该分类节点在 AI 提示词目录中的可见性：
/// - `always_visible = true`  → 始终展示完整节点列表（适合基础/控制流节点）
/// - `always_visible = false` → 仅显示摘要行，AI 需调用 WfQueryNodes 获取完整列表（适合业务节点）
#[derive(Debug, Clone, Copy)]
pub struct CategoryMetadata {
    pub name: &'static str,
    pub description: &'static str,
    pub always_visible: bool,
}

// Collect category metadata in the global inventory registry.
inventory::collect!(CategoryMetadata);

/// 节点权限位字段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodePermissions {
    pub bits: u8,
}

impl NodePermissions {
    pub const NONE: u8 = 0b000000;
    pub const CAN_ADD_INPUT_PIN: u8 = 0b000001;
    pub const CAN_REMOVE_INPUT_PIN: u8 = 0b000010;
    pub const CAN_ADD_OUTPUT_PIN: u8 = 0b000100;
    pub const CAN_REMOVE_OUTPUT_PIN: u8 = 0b001000;
    pub const CAN_EDIT_PIN_TYPE: u8 = 0b010000;
    pub const CAN_EDIT_PIN_NAME: u8 = 0b100000;
    /// 允许具体化通配符类型 (T → String, Array<T> → Array<i64>)
    /// 用于数组构造、数组常量等使用泛型的节点
    pub const CAN_SPECIALIZE_WILDCARD: u8 = 0b10000000;

    pub const fn new(bits: u8) -> Self {
        Self { bits }
    }

    pub fn none() -> Self {
        Self { bits: Self::NONE }
    }

    pub fn has(&self, permission: u8) -> bool {
        (self.bits & permission) != 0
    }
}

impl Default for NodePermissions {
    fn default() -> Self {
        Self::none()
    }
}

// Collect node metadata in the global inventory registry.
inventory::collect!(NodeMetadata);

/// 节点工厂函数类型 - 返回 BlueprintNode 以支持所有节点类型
pub type NodeFactoryFn = fn(String) -> Arc<dyn BlueprintNode + Send + Sync>;

/// 节点工厂
pub struct NodeFactory {
    pub node_type: &'static str,
    pub factory: NodeFactoryFn,
}

// Collect node factories in the global inventory registry.
inventory::collect!(NodeFactory);

/// 动态引脚节点工厂函数类型
/// 用于 permissions 含 CAN_ADD_OUTPUT_PIN 的节点，传入从 JSON 读到的 exec_out 引脚名列表
pub type NodeConfigFactoryFn = fn(String, Vec<String>) -> Arc<dyn BlueprintNode + Send + Sync>;

/// 动态引脚节点工厂（由节点自行注册，覆盖默认 Default 构造）
pub struct NodeConfigFactory {
    pub node_type: &'static str,
    pub factory: NodeConfigFactoryFn,
}

// Collect dynamic-pin node factories in the global inventory registry.
inventory::collect!(NodeConfigFactory);

/// 节点注册表
pub struct NodeRegistry;

impl NodeRegistry {
    fn is_public_node(meta: &NodeMetadata) -> bool {
        !matches!(meta.node_type, "GetVarNode" | "SetVarNode")
    }

    /// 获取所有注册的节点元数据
    pub fn all() -> Vec<&'static NodeMetadata> {
        inventory::iter::<NodeMetadata>()
            .filter(|meta| Self::is_public_node(meta))
            .collect()
    }

    /// 根据名称查找节点（使用 node_type 字段）
    pub fn get(node_type: &str) -> Option<&'static NodeMetadata> {
        inventory::iter::<NodeMetadata>().find(|meta| meta.node_type == node_type)
    }

    /// 内部方法：创建节点实例（仅供 Context 使用）
    pub(crate) fn create_node_internal(
        node_type: &str,
        name: impl Into<String>,
    ) -> Option<Arc<dyn BlueprintNode + Send + Sync>> {
        inventory::iter::<NodeFactory>()
            .find(|factory| factory.node_type == node_type)
            .map(|factory| (factory.factory)(name.into()))
    }

    /// 使用动态引脚创建节点（优先于 create_node_internal，用于 CAN_ADD_OUTPUT_PIN 节点）
    ///
    /// `exec_out_pins`：从 BlueprintJson 的 pins 数组中读取的 ExecOutput 引脚名列表
    pub fn create_node_with_exec_pins(
        node_type: &str,
        name: impl Into<String>,
        exec_out_pins: Vec<String>,
    ) -> Option<Arc<dyn BlueprintNode + Send + Sync>> {
        inventory::iter::<NodeConfigFactory>()
            .find(|factory| factory.node_type == node_type)
            .map(|factory| (factory.factory)(name.into(), exec_out_pins))
    }

    /// 根据分类获取节点
    pub fn by_category(category: &str) -> Vec<&'static NodeMetadata> {
        inventory::iter::<NodeMetadata>()
            .filter(|meta| meta.category == category && Self::is_public_node(meta))
            .collect()
    }

    /// 获取所有分类
    pub fn categories() -> Vec<&'static str> {
        let mut cats: Vec<_> = inventory::iter::<NodeMetadata>()
            .filter(|meta| Self::is_public_node(meta))
            .map(|meta| meta.category)
            .collect();
        cats.sort();
        cats.dedup();
        cats
    }

    /// 根据类别名查询已注册的类别元数据
    pub fn category_meta(category: &str) -> Option<&'static CategoryMetadata> {
        inventory::iter::<CategoryMetadata>().find(|m| m.name == category)
    }

    /// 获取所有已注册的类别元数据
    pub fn all_category_metas() -> Vec<&'static CategoryMetadata> {
        inventory::iter::<CategoryMetadata>().collect()
    }
}
