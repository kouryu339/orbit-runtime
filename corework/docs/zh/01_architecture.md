# 1 架构设计

> 更新时间：2026-05-04  
> 状态：按当前代码更新  
> 关键词：无状态宏定义系统、CacheMap / ScopedCache、ExecutionUnit、运行时工具

## 1.1 核心结论

Corework 不是“缓存-系统-编排三层架构”。当前代码的核心形态是：

```text
业务能力 = 无状态 SystemOperation
能力注册 = 宏 / inventory / SystemRegistry
能力获取 = ExecutionUnit -> Context -> registry/cache/event/world
数据承载 = Cache trait + InMemoryCache + ScopedCache + OrchestrationWorld
上层编排 = workflow / saga / statemachine / module
```

也就是说，`workflow`、`StateMachine`、`Saga`、`Module` 都不是 corework 的基础分层，而是基于同一套执行单元和缓存上下文做出来的上层能力编排封装。

`RAG` 目录目前只有类型和接口骨架，**没有可用的向量检索 / embedding / 索引实现**，不要把它写成已实现能力。

## 1.2 当前代码全图

```text
#[buns_system] / #[define_operation]
        |
        v
SystemOperation / DynamicExecute
        |
        v
SystemRegistry  <--------------------------+
        ^                                   |
        |                                   |
ExecutionUnit -- create_context() --> Context
        |                         |         |
        |                         |         +--> get_dynamic_system()
        |                         +------------> cache / event_bus / telemetry / world
        |
        +--> ScopedCache(scope = unit_id)
        +--> ResourceRegistry(resource access)

上层能力：
  BlueprintWorkflow / BlueprintExecutor
  StateMachine
  Module
  Saga
  RpcStubSystem(runtime dynamic tool)
```

## 1.3 ExecutionUnit 是接入点

`ExecutionUnit` 是所有上层能力获取框架能力的统一入口。它不是 trait，而是一个持有框架引用、作用域缓存和资源声明能力的结构体。

```rust
pub struct ExecutionUnit {
    unit_id: String,
    unit_type: UnitType,
    framework: FrameworkState,
    scoped_cache: Arc<ScopedCache>,
    resource_registry: &'static ResourceRegistry,
}
```

创建执行单元后，业务代码通过它拿到 `Context`：

```rust
use corework::prelude::*;

let framework = FrameworkState::initialize()?;
let unit = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));

let ctx = unit.create_context();
let cache = ctx.get_cache();
let registry = ctx.registry.clone();
```

嵌套编排时可以传入父级 id，共享同一个 `ScopedCache` scope：

```rust
let parent = Arc::new(ExecutionUnit::new_root(UnitType::Blueprint, framework));
let child = ExecutionUnit::new_child(UnitType::StateMachine, &parent)?;
```

## 1.4 CacheMap / ScopedCache 是核心数据面

底层缓存抽象是 `Cache` trait，当前内置实现是 `InMemoryCache`，内部用 `DashMap` 保存 key-value。

```rust
#[async_trait]
pub trait Cache: Send + Sync {
    async fn get_raw(&self, key: &str) -> Result<Option<serde_json::Value>>;
    async fn set_raw(&self, key: &str, value: serde_json::Value, ttl: Option<Duration>) -> Result<()>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn exists(&self, key: &str) -> Result<bool>;
    async fn mget_raw(&self, keys: &[String]) -> Result<Vec<Option<serde_json::Value>>>;
    async fn mset_raw(&self, items: &[(String, serde_json::Value)], ttl: Option<Duration>) -> Result<()>;
    async fn incr(&self, key: &str, delta: i64) -> Result<i64>;
    async fn expire(&self, key: &str, ttl: Duration) -> Result<()>;
    async fn flush(&self) -> Result<()>;
}
```

`ExecutionUnit` 不直接把全局 cache 暴露给业务，而是包装成 `ScopedCache`。所有 key 自动加上 `unit_id:` 前缀，形成执行单元隔离。

```rust
let unit = ExecutionUnit::new_root(UnitType::Module, framework);
unit.cache().set_raw("result", serde_json::json!(42), None).await?;

// 底层真实 key 类似：
// module:550e8400-e29b-41d4-a716-446655440000:result
```

`ScopedCache` 还记录写入过的 key，支持 `dump()` / `restore()`，用于快照和会话恢复类场景。

## 1.5 无状态宏定义系统

业务能力优先写成无状态 `SystemOperation`。状态来自 `Context`，而不是系统结构体字段。

```rust
use corework::prelude::*;

#[buns_system]
pub struct GetUserProfile;

#[async_trait]
impl SystemOperation for GetUserProfile {
    type Input = String;
    type Output = serde_json::Value;
    type Error = FrameworkError;

    async fn execute(&self, user_id: String, ctx: &Context) -> Result<Self::Output> {
        let key = format!("user:{user_id}");
        if let Some(v) = ctx.get_cache().get_raw(&key).await? {
            return Ok(v);
        }

        let value = serde_json::json!({ "id": user_id });
        ctx.get_cache().set_raw(&key, value.clone(), None).await?;
        Ok(value)
    }
}
```

`#[buns_system]` / `#[define_operation]` 通过 `inventory` 把能力注册到 `SystemFactory` / `AISystemFactory` / 节点元数据表中。运行时再由 `SystemRegistry::auto_register_all()` 收集。

## 1.6 动态能力和 RPC 工具

本地宏注册的能力不是唯一入口。运行时工具走 `DynamicExecute`：

```rust
registry.register_dynamic(
    "PythonCtxProbe",
    Arc::new(RpcStubSystem::new(
        "PythonCtxProbe",
        endpoints,
        tools,
        Arc::new(JsonLineRpcToolClient),
    )),
);

let executor = ctx.get_dynamic_system("PythonCtxProbe")?;
let output = executor.execute_dynamic(input, &ctx).await?;
```

这条路径已经用于 RPC 工具 demo：远端工具不能直接拿 Rust `Context`，只能通过受支持的 `workspace.*` HostCall 访问工作区资源。动态 AI 文本仅由宿主通过 runtime FFI 发布。

## 1.7 上层能力定位

| 能力 | 当前定位 | 核心依赖 |
| --- | --- | --- |
| `BlueprintWorkflow` | 节点图编排封装 | `ExecutionUnit`、`ScopedCache`、节点注册表 |
| `StateMachine` | 状态 / 事件 / 转移封装 | `ExecutionUnit`、`post_event` 队列、状态回调 |
| `Module` | `Arc<ExecutionUnit>` 的轻量别名 | `ExecutionUnit` |
| `Saga` | 补偿事务编排 | `Context`、`SagaStep`、`RetryPolicy` |
| `RpcStubSystem` | 运行时远程工具执行器 | `RuntimeToolRegistry`、`RpcEndpointRegistry`、`DynamicExecute` |

它们不是从低到高的固定三层，而是并列消费同一套核心能力。

## 1.8 RAG 当前状态

`src/rag` 当前包含：

```text
src/rag/types.rs      Document / QueryResult / RagConfig
src/rag/store.rs      DocumentStore trait
src/rag/retriever.rs  Retriever trait
```

未实现的部分：

- embedding 生成
- 向量索引
- 文档切分
- 持久化向量库
- 与 AI prompt 的检索注入链路

因此文档中只能说“预留接口 / 骨架”，不能说 Corework 已经具备 RAG 能力。

## 1.9 推荐心智模型

写 corework 代码时按这个顺序想：

1. 原子业务能力写成无状态 `SystemOperation`。
2. 数据需要短生命周期隔离时放 `Context.cache` / `ScopedCache`。
3. 跨执行单元共享且需要权限边界时放 `OrchestrationWorld`。
4. 需要 AI 工具或蓝图节点时用 `#[define_operation]` 生成注册元数据。
5. 需要外部语言扩展时注册 `RpcStubSystem` / runtime tool。
6. 需要复杂流程时再选择 workflow、状态机、Saga 或 Module。

下一篇：[02_缓存系统详解.md](02_缓存系统详解.md)
