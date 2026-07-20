# 3 装饰器与注册系统

> 更新时间：2026-05-04  
> 状态：按当前代码更新

## 3.1 结论

Corework 的“能力注册”主要由两条路径组成：

```text
编译期本地能力：
  #[buns_system] / #[define_operation]
    -> inventory
    -> SystemFactory / AISystemFactory / NodeFactory
    -> SystemRegistry::auto_register_all()

运行时远程能力：
  RuntimeToolMetadata
    -> RuntimeToolRegistry
    -> RpcStubSystem
    -> SystemRegistry::register_dynamic()
```

本地系统推荐无状态。运行时 RPC 工具不经过 Rust 宏，也不需要 Rust concrete type。

## 3.2 SystemOperation

系统是无状态业务逻辑单元：

```rust
#[async_trait]
pub trait SystemOperation: Send + Sync {
    type Input: Send + Sync;
    type Output: Send + Sync;
    type Error: Into<FrameworkError> + Send;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> std::result::Result<Self::Output, Self::Error>;
}
```

最小实现：

```rust
use corework::prelude::*;

#[buns_system]
pub struct Echo;

#[async_trait]
impl SystemOperation for Echo {
    type Input = String;
    type Output = String;
    type Error = FrameworkError;

    async fn execute(&self, input: String, _ctx: &Context) -> Result<String> {
        Ok(input)
    }
}
```

## 3.3 `#[buns_system]`

`#[buns_system]` 做两类注册：

- `SystemFactory`：普通系统注册。
- `AISystemFactory`：只有声明了 `params {}` 的系统才会成为 AI 工具。

### 本地工具显示名

对于每一个本地 AI 工具，`name` 是稳定的机器调用标识，`display_name` 是面向用户的
输入输出关系模板。模板使用单层花括号引用已声明的参数和输出字段，例如：

```text
等待{timeout_ms}毫秒并返回{wake_reason}
打开工作流{workflow_id}并返回草稿{draft_name}
将内容{content}写入{file_name}并返回路径{path}
```

本地工具声明的每个输入和输出都必须以 `{field}` 出现在模板中，模板也不能引用未声明
字段。没有声明字段的工具可以使用纯动作句。这条约定只适用于本地工具；RPC 工具的
显示名继续由各 Endpoint 的 descriptor 自行定义。

AI 工具示例：

```rust
#[buns_system(
    "ReadNote",
    description = "读取一条笔记",
    params {
        id: "笔记 ID",
    },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false,
)]
pub struct ReadNote;

#[async_trait]
impl SystemOperation for ReadNote {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput> {
        let args = input.safe_parse_args()
            .map_err(|e| FrameworkError::InvalidOperation(e.to_ai))?;
        let id = args.get_required("id")?;

        let key = format!("note:{id}");
        let note = ctx.cache.get_raw(&key).await?.unwrap_or(serde_json::Value::Null);

        Ok(AIOutput::success(note, format!("已读取笔记 {id}")))
    }
}
```

AI 工具的输入输出统一为：

```rust
pub struct AIInput {
    pub input: String, // CLI 风格参数字符串
}

pub struct AIOutput {
    pub result: serde_json::Value,
    pub to_ai: String,
    pub error_code: i32,
}
```

`to_ai` 是写回 AI 工具结果通道的文本，必须清晰描述本次调用结果。

## 3.4 `#[define_operation]`

当同一个能力既要给 AI 调用，又要进 workflow 节点列表时，优先用 `#[define_operation]`。

```rust
#[define_operation(
    name = "ClickElement",
    description = "点击指定元素",
    category = "Browser",
    params {
        selector: "CSS 选择器",
    },
    outputs {
        success: bool,
    },
    exec_in = true,
    exec_out = true,
    flags {
        destructive = true,
        readonly = false,
        idempotent = false,
        open_world = true
    },
)]
pub struct ClickElement;
```

它会同时生成：

- `SystemFactory`
- `AISystemFactory`
- `DynamicExecute`
- workflow `NodeMetadata`
- workflow `NodeFactory`

如果只希望生成系统，不生成节点：

```rust
#[define_operation(
    name = "InternalTool",
    description = "内部工具",
    system_only,
)]
pub struct InternalTool;
```

工作流节点的 `display_name` 同时承担可读模板语义。Pure 节点使用单层花括号引用全部
数据输入，例如 `{A}+{B}` 或 `{Value}是否包含{Pattern}`；控制节点使用
`根据{Condition}选择分支` 这类动作模板。模板引用的字段必须对应真实引脚。

## 3.5 SystemRegistry

`SystemRegistry` 当前保存两张表：

```rust
pub struct SystemRegistry {
    operations: Arc<RwLock<HashMap<String, Arc<dyn Any + Send + Sync>>>>,
    dynamic_executors: Arc<RwLock<HashMap<String, Arc<dyn DynamicExecute>>>>,
}
```

编译期注册收集：

```rust
let registry = SystemRegistry::new();
registry.auto_register_all();
```

类型化获取：

```rust
let op: Option<Arc<MySystem>> = registry.get("MySystem");
```

动态执行获取：

```rust
let dyn_exec = registry.get_dynamic("ClickElement");
```

运行时注册：

```rust
registry.register_dynamic(
    "PythonCtxProbe",
    Arc::new(RpcStubSystem::new("PythonCtxProbe", endpoints, tools, client)),
);
```

## 3.6 RuntimeToolMetadata

RPC 工具的元数据是 owned 结构体，不能用 `&'static str`：

```rust
pub struct RuntimeToolMetadata {
    pub name: String,
    pub description: String,
    pub parameters: Vec<RuntimeAIParameter>,
    pub outputs: Vec<RuntimeAIOutputField>,
    pub destructive: bool,
    pub readonly: bool,
    pub idempotent: bool,
    pub open_world: bool,
    pub secret: bool,
    pub required_capabilities: Vec<String>,
    pub endpoint_id: String,
    pub service: String,
    pub method: String,
}
```

这是 RPC 工具进入 AI prompt、参数校验和运行时执行链路的基础。

## 3.7 register_category!

节点分类通过宏注册：

```rust
corework::register_category!(
    name = "Browser",
    description = "浏览器自动化节点",
    always_visible = false,
);
```

`always_visible = true` 适合控制流、基础数学等常驻节点。业务节点一般用 `false`，避免一次性塞进过多上下文。

## 3.8 推荐写法

普通业务能力：

```rust
#[buns_system]
pub struct QueryInventory;
```

AI 工具：

```rust
#[buns_system(
    "QueryInventory",
    description = "查询库存",
    params { product_id: "商品 ID" },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false,
)]
pub struct QueryInventory;
```

AI 工具 + workflow 节点：

```rust
#[define_operation(
    name = "QueryInventory",
    description = "查询库存",
    category = "Inventory",
    params { product_id: "商品 ID" },
    outputs { stock: i64 },
)]
pub struct QueryInventory;
```

运行时 RPC 工具：

```rust
tool_registry.insert(metadata)?;
system_registry.register_dynamic(
    metadata.name.clone(),
    Arc::new(RpcStubSystem::new(metadata.name, endpoints, tools, client)),
);
```

下一篇：[04_蓝图系统详解.md](04_蓝图系统详解.md)
