# 2 缓存系统详解

> 更新时间：2026-05-04  
> 状态：按当前代码更新

## 2.1 结论

Corework 的数据核心不是抽象的“数据层”，而是两套明确的存储面：

```text
Cache trait / InMemoryCache
  -> serde_json::Value key-value
  -> DashMap
  -> TTL / mget / mset / incr / expire

ScopedCache
  -> 包装 Cache
  -> 自动给 key 加 scope_id 前缀
  -> 跟踪当前 scope 写过的 key
  -> cleanup / dump / restore

OrchestrationWorld
  -> 全局 Resource
  -> Instance 数据
  -> 适合跨执行单元共享
```

日常执行路径优先使用 `Context.cache`，它通常已经是 `ExecutionUnit` 创建出来的 `ScopedCache`。

## 2.2 Cache trait

当前接口定义在 `src/cache.rs`：

```rust
#[async_trait]
pub trait Cache: Send + Sync {
    async fn get_raw(&self, key: &str) -> Result<Option<serde_json::Value>>;
    async fn set_raw(&self, key: &str, value: serde_json::Value, ttl: Option<Duration>) -> Result<()>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn exists(&self, key: &str) -> Result<bool>;
    async fn mget_raw(&self, keys: &[String]) -> Result<Vec<Option<serde_json::Value>>>;
    async fn mset_raw(&self, items: &[(String, serde_json::Value)], ttl: Option<Duration>) -> Result<()>;
    async fn dump_raw(&self) -> Result<HashMap<String, serde_json::Value>>;
    async fn incr(&self, key: &str, delta: i64) -> Result<i64>;
    async fn expire(&self, key: &str, ttl: Duration) -> Result<()>;
    async fn flush(&self) -> Result<()>;
}
```

`dump_raw()` 用于可持久化快照导出。默认实现返回不支持；`ScopedCache` 返回当前 scope 中已追踪的逻辑 key，`InMemoryCache` 返回未过期 key 并去掉 `CacheConfig.key_prefix`。

`CacheExt` 在此基础上提供类型安全读写：

```rust
use corework::prelude::*;

let cache = InMemoryCache::new();

cache.set("answer", &42_i64, None).await?;
let answer: Option<i64> = cache.get("answer").await?;
```

对象字段路径也在 `CacheExt` 中：

```rust
let sub_id: Option<String> = cache.get_field("sub_question", "sub_id").await?;
cache.set_field("sub_question", "status", &"done").await?;
```

## 2.3 InMemoryCache

`InMemoryCache` 是当前内置实现：

```rust
pub struct InMemoryCache {
    store: Arc<DashMap<String, CacheEntry>>,
    config: CacheConfig,
}

#[derive(Clone)]
struct CacheEntry {
    value: serde_json::Value,
    expires_at: Option<DateTime<Utc>>,
}
```

写入时会合并调用处传入的 TTL 和 `CacheConfig.default_ttl`：

```rust
let cache = InMemoryCache::with_config(CacheConfig {
    default_ttl: Some(Duration::from_secs(3600)),
    key_prefix: "app".to_string(),
    enable_compression: false,
    max_value_size: 1024 * 1024,
});

cache.set_raw("session", serde_json::json!({ "id": 1 }), None).await?;
```

注意：`enable_compression` 和 `max_value_size` 当前是配置字段，文档不要写成已有完整压缩和大小拦截实现。

## 2.4 ScopedCache

`ScopedCache` 是 `ExecutionUnit` 的关键数据隔离机制。它包装任意 `Cache`，把所有 key 转成：

```text
{scope_id}:{key}
```

示例：

```rust
let base = Arc::new(InMemoryCache::new());
let scoped = ScopedCache::new(base.clone(), "workflow:order-1");

scoped.set("step_result", &true, None).await?;

// 底层等价于写入：
// workflow:order-1:step_result
```

它会跟踪写入过的裸 key：

```rust
let snapshot = scoped.dump_raw().await?;
scoped.cleanup().await?;
scoped.restore(snapshot).await?;
```

这对会话快照、工具执行中间状态、子执行单元共享父 scope 都很重要。

## 2.5 ExecutionUnit 如何创建缓存上下文

`ExecutionUnit::new_root()` 从 `FrameworkState` 里拿到底层 cache，再包成 local `ScopedCache`；子执行单元必须通过 `ExecutionUnit::new_child()` 绑定真实父节点：

```rust
let global_ctx = framework.create_context();
let scoped_cache = Arc::new(ScopedCache::new(
    global_ctx.cache.clone(),
    unit_id.clone(),
));
```

业务系统只看到 `Context`：

```rust
#[async_trait]
impl SystemOperation for MySystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput> {
        ctx.cache
            .set_raw("last_input", serde_json::json!(input.input), None)
            .await?;

        Ok(AIOutput::success(
            serde_json::json!({ "ok": true }),
            "已写入当前执行单元缓存",
        ))
    }
}
```

## 2.6 World 与 Cache 的选择

| 数据 | 推荐位置 | 原因 |
| --- | --- | --- |
| 节点输出、中间值、当前状态 | `Context.cache` / `ScopedCache` | 自动按执行单元隔离 |
| 状态机 `current_state` | `ExecutionUnit.cache()` | 状态机实例私有 |
| 宿主动态文本 `host_dynamic_snapshots` | Runtime FFI 写入的 Agent cache | 当前会话 / Agent 范围 |
| 全局配置、共享资源 | `OrchestrationWorld` | 跨执行单元共享 |
| 需要权限声明的共享资源 | `ExecutionUnit.get_resource/set_resource` | 走 `ResourceRegistry` 检查 |

## 2.7 World 资源访问

直接操作 world：

```rust
let world = unit.world();
world.set_resource("app:config", &config, None)?;
let config: Option<AppConfig> = world.get_resource("app:config")?;
```

带执行单元权限检查：

```rust
unit.declare_resource_access("app:config", AccessMode::Owner)?;
unit.set_resource("app:config", &config, None)?;

let config: Option<AppConfig> = unit.get_resource("app:config")?;
```

`Owner` 拥有读写能力。非 owner 需要由 owner 调用 `grant_access_to()`。

## 2.8 最佳实践

- 系统内部不要持有业务状态，把状态放进 `ctx.cache` 或 `world`。
- 执行单元私有数据优先写裸 key，例如 `"current_state"`，让 `ScopedCache` 自动补 scope。
- 需要导出 / 恢复的状态，必须通过 `set_raw` / `mset_raw` 写入，确保被 `tracked_keys` 记录。
- `flush()` 在 `ScopedCache` 上只清理当前 scope，不是清空全局 cache。
- 跨进程 RPC 工具只能通过受支持的 `workspace.*` HostCall 访问工作区资源，不要假设能拿到 Rust `Context`；动态 AI 文本仅由宿主通过 FFI 发布。

下一篇：[03_装饰器与注册系统.md](03_装饰器与注册系统.md)
