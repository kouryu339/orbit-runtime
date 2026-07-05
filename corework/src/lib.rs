//! # Corework
//!
//!
//! ## 当前核心模型
//!
//! ```text
//! #[buns_system] / #[define_operation]
//!   -> SystemOperation / DynamicExecute
//!   -> SystemRegistry
//!   -> ExecutionUnit::create_context()
//!   -> ScopedCache + EventBus + Telemetry + World
//!
//! 上层能力：
//!   BlueprintWorkflow / StateMachine / Saga / Module / RpcStubSystem
//! ```
//!
//! `rag` 模块目前只保留类型和 trait 骨架，尚未实现可用的 embedding、向量索引或检索注入链路。

// 允许宏在本 crate 中使用 ::corework:: 路径
#![allow(
    clippy::empty_docs,
    clippy::if_same_then_else,
    clippy::inherent_to_string,
    clippy::large_enum_variant,
    clippy::module_inception,
    clippy::needless_range_loop,
    clippy::obfuscated_if_else,
    clippy::only_used_in_recursion,
    clippy::should_implement_trait,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

extern crate self as corework;

// 重新导出 SystemFactory 和 ModelTypeFactory
pub use data_type::ModelTypeFactory;
pub use system::SystemFactory;

// 重新导出装饰器宏（框架核心能力）
pub use buns_macros::{
    buns_enum, buns_model, buns_orchestration, buns_system, define_operation, register_node,
};

pub mod ai_system; // AI 可调用系统
pub mod cache;
pub mod common_tools;
pub mod data_type;
pub mod ecs;
pub mod error;
pub mod event;
pub mod event_line;
pub mod execution_unit; // 执行单元基础设施入口
pub mod hierarchical_cache;
pub mod instance;
pub mod module; // 轻量模块封装：Arc<ExecutionUnit>
pub mod monitoring;
pub mod orchestration;
pub mod rag; // RAG 类型/trait 骨架，尚未实现检索链路
pub mod retry;
pub mod rpc_proto;
pub mod rpc_tool;
pub mod runtime_state;
pub mod saga;
pub mod scoped_cache;
pub mod statemachine; // 基于 ExecutionUnit 的状态机封装
pub mod system;
pub mod workflow; // 基于 ExecutionUnit 的蓝图编排
pub mod workspace; // 文件副本安全机制（Copy-on-Read）
pub mod world;

/// 便捷导入模块
pub mod prelude {
    pub use crate::cache::{
        create_cache_backend, Cache, CacheBackendConfig, CacheConfig, CacheExt, CacheValue,
        InMemoryCache,
    };
    pub use crate::data_type::{type_name_of, DataType, DataTypeRegistry, ModelTypeFactory};
    pub use crate::ecs::{
        EcsCommand, EcsUnitSnapshot, EcsWorld, HierarchyComponent, LifecycleComponent,
        ResourceAccessComponent, ScopeComponent, SpawnUnitCommand, UnitEntityId,
        UnitIdentityComponent, UnitLifecycleStatus,
    };
    pub use crate::error::{FrameworkError, Result};
    pub use crate::event::{Event, EventBus, EventHandler, InMemoryEventBus};
    pub use crate::event_line::{EventLineAccess, EventLineHandle, EventLinePolicy};
    pub use crate::hierarchical_cache::HierarchicalCache;
    pub use crate::instance::InstanceHandle;
    pub use crate::monitoring::{Metrics, NoopTelemetry, Telemetry};
    pub use crate::orchestration::{Context, Orchestrator, OrchestratorBuilder};
    pub use crate::rpc_tool::{
        AgentToolRequest, GrpcRpcToolClient, GrpcRpcToolDiscoveryClient, JsonLineRpcToolClient,
        JsonLineRpcToolDiscoveryClient, RemoteAIOutput, RpcEndpointInfo, RpcEndpointRegistry,
        RpcStubSystem, RpcToolClient, RuntimeAIOutputField, RuntimeAIParameter,
        RuntimeToolMetadata, RuntimeToolRegistry,
    };
    pub use crate::runtime_state::{
        create_runtime_state_store, EcsRuntimeStateStore, EventLineStateSnapshot, EventStateStore,
        HybridRuntimeStateStore, KeyValueStateStore, MapRuntimeStateStore, ResourceAccessSnapshot,
        ResourceStateStore, RuntimeStateBackendKind, RuntimeStateConfig, RuntimeStateStore,
        SharedComponentStateSnapshot, SharedComponentStateStore, StateScope, UnitStateStore,
    };
    pub use crate::scoped_cache::{ScopedCache, ScopedCacheStats};
    pub use crate::system::{AutoRegisterSystem, SystemFactory, SystemOperation, SystemRegistry};

    // 装饰器宏（框架核心）
    pub use crate::{buns_model, buns_orchestration, buns_system, define_operation};

    // 工作流模块（蓝图工作流）
    pub use crate::workflow::{
        BlueprintExecutor, BlueprintNode, BlueprintWorkflow, BlueprintWorkflowBuilder, BranchNode,
        Connection, DataValue, EntryNode, ForLoopNode, NodeOutput, Pin, PinCacheMapping,
        PinDirection, PinType, PureFunctionNode, SystemNode, SystemNodeBuilder, TaskNode,
    };

    pub use crate::workflow::dynamic_node::{
        DynamicExecute, DynamicSystemNode, DynamicSystemNodeBuilder,
    };

    // 节点注册相关类型（供 register_node / register_category 宏使用）
    pub use crate::workflow::registry::{
        CategoryMetadata, NodeFactory, NodeMetadata, NodePermissions, PinKind, PinMetadata,
    };

    pub use crate::statemachine::{
        ConditionFn, ConditionalTransition, FnState, SimpleTransition, State as StateMachineState,
        StateEvent, StateFn, StateMachine as StateMachineL2, StateMachineBuilder,
        StateMachineEventQueue, Transition as StateMachineTransition, TransitionFn,
        TransitionRecord, SM_STATE_ENTER, SM_STATE_EXIT,
    };

    pub use crate::world::OrchestrationWorld;

    pub use crate::world::FrameworkState;

    // 业务协调封装
    pub use crate::retry::RetryPolicy;
    pub use crate::saga::{Saga, SagaBuilder, SagaStep, SimpleSaga};

    // 执行单元基础设施入口
    pub use crate::execution_unit::{
        AccessMode, ExecutionUnit, ResourceClaim, ResourceRegistry, UnitType,
    };

    // 简化模块封装（直接使用 ExecutionUnit）
    pub use crate::module::{create_module, create_module_with_framework, Module};

    pub use async_trait::async_trait;
    pub use serde::{Deserialize, Serialize};
}

// ============================================================================
// register_category! — 注册节点分类元数据
// ============================================================================

/// 注册节点分类元数据到全局注册表。
///
/// 控制该分类的节点在 AI 提示词目录（节点目录）中的可见策略：
/// - `always_visible = true`  → 节点列表始终出现在 Tier 1（适合控制流、基础运算等常用节点）
/// - `always_visible = false` → Tier 2 按需加载，AI 调用 `WfQueryNodes` 后才展开（适合业务节点）
///
/// # 示例
/// ```rust
/// // 基础节点：始终可见
/// corework::register_category!(
///     name = "Control Flow",
///     description = "控制工作流执行顺序：条件分支、循环、子图",
///     always_visible = true,
/// );
///
/// // 业务节点：按需加载
/// corework::register_category!(
///     name = "Audio",
///     description = "音频处理：格式转换、裁剪、合并、音量调整",
///     always_visible = false,
/// );
/// ```
#[macro_export]
macro_rules! register_category {
    (
        name         = $name:expr,
        description  = $description:expr,
        always_visible = $av:expr $(,)?
    ) => {
        ::inventory::submit! {
            ::corework::workflow::registry::CategoryMetadata {
                name:          $name,
                description:   $description,
                always_visible: $av,
            }
        }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
