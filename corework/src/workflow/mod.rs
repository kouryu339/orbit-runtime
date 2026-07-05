//! 工作流模块
//!

// 新的模块化架构
pub mod blueprint_json;
pub mod blueprint_loader;
pub mod builder;
pub mod chain_ast;
pub mod chain_compiler;
pub mod chain_compiler_v2;
pub mod chain_decompiler;
pub mod chain_id;
pub mod compiler;
pub mod core;
pub mod execution;
pub mod interface;
pub mod nodes;
pub mod pure_function_codec;
pub mod registry;
pub mod split_pin;
pub mod syntax_lex;
pub mod type_inference;
pub mod workflow; // 可复用工作流实例
pub mod workflows;

// 旧模块（向后兼容）
pub mod blueprint;
pub mod control_nodes;
pub mod data_nodes;
pub mod dynamic_node;

// 重新导出蓝图相关类型
pub use blueprint::{
    BlueprintExecutor, BlueprintNode, BlueprintWorkflow, BlueprintWorkflowBuilder, BranchNode,
    Connection, DataValue, EntryNode, ForLoopNode, NodeOutput, Pin, PinCacheMapping,
    PinContainerType, PinDirection, PinType, PureFunctionNode, SystemNode, SystemNodeBuilder,
    TaskNode,
};
pub use dynamic_node::{DynamicSystemNode, DynamicSystemNodeBuilder};

// 重新导出控制流节点
pub use control_nodes::{
    BranchNode as ControlBranchNode, DelayNode, DoNNode, ForEachNode,
    ForLoopNode as ControlForLoopNode, SelectNode, WhileLoopNode,
};

// 重新导出数据操作节点
pub use data_nodes::{
    // 数学运算
    AddNode,
    // 逻辑
    AndNode,
    // 字符串
    ConcatNode,
    EqualNode,
    FormatStringNode,
    // 比较
    GreaterNode,
    MakeBoolNode,
    // 常量
    MakeIntNode,
    MultiplyNode,
    NotNode,
    OrNode,
    // 类型转换
    ToIntNode,
    ToStringNode,
};

pub use blueprint_json::{
    BlueprintJson, BlueprintNodeJson, BlueprintVariable, BlueprintVisibility, CommentBox,
    CommentSize, ConnectionJson, NodePin, NodePosition, PinMetadata,
};
pub use blueprint_loader::{BlueprintLoader, BlueprintSaver, LoadedBlueprint};
pub use workflows::WorkflowsModule;

// 为了避免名称冲突，BlueprintMetadata 通过模块路径访问：
// - blueprint_json::BlueprintMetadata - 蓝图JSON格式的元数据（包含inputs/outputs）
