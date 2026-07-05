# 6 业务开发指南

> 更新时间：2026-05-04  
> 状态：按当前代码更新  
> 原则：代码优先，系统无状态，数据进 Context / Cache / World

## 6.1 写一个普通系统

```rust
use corework::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: String,
    pub stock: i64,
}

#[buns_system]
pub struct InventoryRepository;

#[async_trait]
impl SystemOperation for InventoryRepository {
    type Input = String;
    type Output = Option<Product>;
    type Error = FrameworkError;

    async fn execute(&self, product_id: String, ctx: &Context) -> Result<Option<Product>> {
        let key = format!("product:{product_id}");

        if let Some(value) = ctx.cache.get::<Product>(&key).await? {
            return Ok(Some(value));
        }

        Ok(None)
    }
}
```

系统结构体不保存连接、状态、当前用户等业务数据。这些数据来自 `ctx` 或输入参数。

## 6.2 写一个 AI 工具

AI 工具使用 `AIInput` / `AIOutput`：

```rust
#[buns_system(
    "GetProduct",
    description = "按商品 ID 查询商品",
    params {
        product_id: "商品 ID",
    },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false,
)]
pub struct GetProduct;

#[async_trait]
impl SystemOperation for GetProduct {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(output) => return Ok(output),
        };
        let product_id = match args.safe_require("product_id") {
            Ok(v) => v,
            Err(output) => return Ok(output),
        };

        let key = format!("product:{product_id}");
        let product = ctx.cache.get_raw(&key).await?;

        Ok(AIOutput::success(
            product.unwrap_or(serde_json::Value::Null),
            format!("已查询商品 {product_id}"),
        ))
    }
}
```

推荐在工具内部用 `safe_parse_args()` / `safe_require()`，把参数错误转成 `AIOutput`，这样 AI 能看到可修正的错误。

## 6.3 同时生成 AI 工具和蓝图节点

```rust
#[define_operation(
    name = "GetProduct",
    description = "按商品 ID 查询商品",
    category = "Inventory",
    params {
        product_id: "商品 ID",
    },
    outputs {
        exists: bool,
        stock: i64,
    },
    flags {
        destructive = false,
        readonly = true,
        idempotent = true,
        open_world = false
    },
)]
pub struct GetProduct;
```

适合一份能力同时给 AI、workflow 编辑器、运行时系统表使用。

## 6.4 在代码里直接调用系统

```rust
let framework = FrameworkState::initialize()?;
let unit = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
let ctx = unit.create_context();

let repo = InventoryRepository;
let product = repo.execute("p-1".to_string(), &ctx).await?;
```

## 6.5 使用执行单元缓存

```rust
let unit = ExecutionUnit::new_root(UnitType::Module, framework);

unit.cache()
    .set("current_order", &"order-1".to_string(), None)
    .await?;

let order: Option<String> = unit.cache().get("current_order").await?;
```

真实 key 会被 `ScopedCache` 加上 `unit.id()` 前缀。

## 6.6 使用 World 共享资源

```rust
unit.declare_resource_access("app:config", AccessMode::Owner)?;
unit.set_resource("app:config", &config, None)?;

let config: Option<AppConfig> = unit.get_resource("app:config")?;
```

只有跨执行单元共享、且需要资源权限边界的数据才放 World。节点中间值、工具临时结果不要放 World。

## 6.7 注册运行时 RPC 工具

```rust
let endpoints = Arc::new(RpcEndpointRegistry::new());
endpoints.insert(RpcEndpointInfo {
    endpoint_id: "python-demo".to_string(),
    address: "127.0.0.1:58081".to_string(),
    timeout_ms: 10_000,
})?;

let tools = Arc::new(RuntimeToolRegistry::new());
tools.insert(RuntimeToolMetadata {
    name: "PythonResolvePath".to_string(),
    description: "解析受宿主管理的工作文件路径".to_string(),
    parameters: vec![RuntimeAIParameter {
        name: "path".to_string(),
        param_type: "String".to_string(),
        required: true,
        default_value: None,
        description: "源文件路径".to_string(),
    }],
    outputs: vec![],
    destructive: false,
    readonly: false,
    idempotent: false,
    open_world: true,
    secret: false,
    required_capabilities: vec!["workspace.resolve_path".to_string()],
    endpoint_id: "python-demo".to_string(),
    service: "json-lines-test".to_string(),
    method: "execute".to_string(),
})?;

ctx.registry.register_dynamic(
    "PythonResolvePath",
    Arc::new(RpcStubSystem::new(
        "PythonResolvePath",
        endpoints,
        tools,
        Arc::new(JsonLineRpcToolClient),
    )),
);
```

`snapshot.get`、`snapshot.put` 和 `allowed_snapshot_prefixes` 已删除，旧 RPC
工具无法兼容运行。需要注入动态 AI 上下文时，应由宿主通过 runtime FFI 发布
对应 agent 的纯文本字段。

## 6.8 当前 RAG 不可用

不要在业务里依赖 `corework::rag` 做检索。当前只有 trait / 类型骨架，没有 embedding、向量库和检索注入链路。

可以定义自己的系统先接外部检索服务：

```rust
#[buns_system(
    "SearchKnowledge",
    description = "调用外部知识库检索",
    params { query: "查询文本" },
    destructive = false,
    readonly = true,
    idempotent = false,
    open_world = true,
)]
pub struct SearchKnowledge;
```

## 6.9 常见错误

不要这样做：

```rust
pub struct BadSystem {
    pub current_user: String,
    pub last_result: Option<String>,
}
```

改成：

```rust
pub struct GoodSystem;

// current_user 从 input 或 ctx.cache 读取
// last_result 写回 ctx.cache
```

不要把 `workflow`、`StateMachine`、`Saga` 当成核心分层。它们只是消费执行单元能力的编排封装。

下一篇：[09_RPC工具协议v1.md](09_RPC工具协议v1.md)
