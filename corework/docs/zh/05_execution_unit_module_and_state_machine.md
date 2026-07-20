# 5 执行单元、Module、StateMachine 与 Saga

> 更新时间：2026-05-04  
> 状态：按当前代码更新

## 5.1 定位

`ExecutionUnit` 是核心接入点。`Module`、`StateMachine`、`Saga` 都是基于它或 `Context` 的上层封装。

```text
ExecutionUnit
  -> create_context()
  -> Context
  -> cache / event_bus / telemetry / registry / world

Module       = Arc<ExecutionUnit>
StateMachine = 持有 Arc<ExecutionUnit> 的状态/事件封装
Saga         = 使用 Context 顺序执行步骤和补偿
```

不要把这些叫成 L2 / L3 层。

## 5.2 ExecutionUnit

创建：

```rust
let framework = FrameworkState::initialize()?;
let unit = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
```

访问能力：

```rust
let cache = unit.cache();
let world = unit.world();
let registry = unit.registry();
let event_bus = unit.event_bus();
let ctx = unit.create_context();
```

资源权限：

```rust
unit.declare_resource_access("scores:config", AccessMode::Owner)?;
unit.set_resource("scores:config", &config, None)?;

let config: Option<ScoresConfig> = unit.get_resource("scores:config")?;
```

授权给其他单元：

```rust
unit.grant_access_to(
    "scores:config",
    "module:browser",
    AccessMode::Read,
)?;
```

## 5.3 Module

当前 `Module` 是轻量别名：

```rust
pub type Module = Arc<ExecutionUnit>;
```

创建：

```rust
let module = create_module("scores")?;
module.declare_resource_access("scores:refs", AccessMode::Owner)?;
```

注意：`create_module(module_id)` 当前不会把 `module_id` 写入 `unit_id`，`ExecutionUnit::new_root()` 仍会生成 `module:{uuid}`。文档和业务代码不要假设 `module_id` 就是 scope id。

如果要和父执行单元共享 cache scope：

```rust
let child = create_child_module("child", &parent)?;
```

## 5.4 StateMachine

状态机持有一个 `Arc<ExecutionUnit>`，当前状态会写入它的 scoped cache：

```rust
self.unit.cache().set("current_state", &initial, None).await?;
```

构建：

```rust
let sm = StateMachine::builder("order")
    .add_state(Box::new(FnState::new("created")))
    .add_state(Box::new(FnState::terminal("done")))
    .initial_state("created")
    .build()
    .await?;

sm.start().await?;
```

推荐用 `post_event()` 处理并发事件：

```rust
sm.post_event("PAUSE");
sm.process_events().await?;
```

`send_event()` 是立即同步转移。当前状态的 `on_enter` 仍在 await 时使用它，可能造成状态回调并发执行。并发场景优先用 `post_event()`。

状态函数拿到执行单元后，可以访问 cache / registry / world：

```rust
let thinking = FnState::new("thinking")
    .on_enter(|unit| Box::pin(async move {
        unit.cache()
            .set("status", &"thinking", None)
            .await?;

        let ctx = unit.create_context();
        let tool = ctx.get_dynamic_system("SearchFile")?;
        // ...
        Ok(())
    }));
```

## 5.5 Saga

`Saga` 是补偿事务封装，不是 corework 核心层。它通过 `Context` 执行每一步。

```rust
struct CreateOrderStep;

#[async_trait]
impl SagaStep for CreateOrderStep {
    fn name(&self) -> &str {
        "create_order"
    }

    async fn execute(&self, ctx: &Context) -> Result<()> {
        ctx.cache.set("order:created", &true, None).await?;
        Ok(())
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        ctx.cache.delete("order:created").await?;
        Ok(())
    }
}
```

构建并执行：

```rust
let saga = SagaBuilder::new("order_saga", world)
    .add_step(CreateOrderStep)
    .with_retry(RetryPolicy::default())
    .build();

saga.execute(&ctx).await?;
```

失败时 `SimpleSaga` 会按已完成步骤的逆序调用 `compensate()`。

## 5.6 选择建议

| 需求 | 用什么 |
| --- | --- |
| 原子业务能力 | `SystemOperation` |
| 能力需要无状态注册和 AI/tool 调用 | `#[buns_system]` / `#[define_operation]` |
| 临时状态和中间值 | `ExecutionUnit.cache()` / `Context.cache` |
| 简单业务模块边界 | `Module = Arc<ExecutionUnit>` |
| 状态驱动流程 | `StateMachine` |
| 补偿事务 | `Saga` |
| 可视化或 AI 生成流程 | `BlueprintWorkflow` |

下一篇：[06_业务开发指南.md](06_业务开发指南.md)
