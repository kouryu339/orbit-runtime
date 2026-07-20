# 4 蓝图系统详解

> 更新时间：2026-05-04  
> 状态：按当前代码更新

## 4.1 定位

`workflow` 是建立在 `ExecutionUnit + ScopedCache + SystemRegistry` 之上的上层编排能力。它不是核心分层，也不是系统的父层。它的职责是执行节点图，把节点输入输出通过 cache 串起来。

当前主要模块：

```text
src/workflow/blueprint.rs              BlueprintWorkflow
src/workflow/execution/executor.rs     BlueprintExecutor
src/workflow/execution/execution_context.rs
src/workflow/core/pin.rs               Pin / PinType
src/workflow/core/data_value.rs        DataValue
src/workflow/registry/node_registry.rs NodeFactory / NodeMetadata
src/workflow/dynamic_node.rs           DynamicSystemNode / DynamicExecute
src/workflow/chain_compiler*.rs        Chain Text -> Blueprint
src/workflow/chain_decompiler.rs       Blueprint -> Chain Text
```

## 4.2 执行路径

```text
BlueprintWorkflow
  -> BlueprintExecutor
  -> ExecutionContext
  -> BlueprintNode::execute()
  -> ctx.inner(): Context
  -> ScopedCache / SystemRegistry / EventBus / World
```

节点之间不要直接共享 Rust 变量，数据通过 cache key 传递。

```rust
ctx.cache()
    .set_raw("check_stock::available", serde_json::json!(true), None)
    .await?;
```

由于 `ExecutionContext` 来自执行单元，cache 实际上是带 scope 的 `ScopedCache`。

## 4.3 BlueprintNode

节点实现关注三件事：

- 描述 pins。
- 声明 pin 到 cache key 的映射。
- 在 `execute()` 中读输入、写输出。

```rust
use corework::prelude::*;
use corework::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

pub struct CheckStockNode {
    input_mappings: Vec<PinCacheMapping>,
    output_mappings: Vec<PinCacheMapping>,
}

impl Default for CheckStockNode {
    fn default() -> Self {
        Self {
            input_mappings: vec![
                PinCacheMapping::new("ProductID", "input::product_id", "String"),
            ],
            output_mappings: vec![
                PinCacheMapping::new("Available", "stock::available", "bool"),
                PinCacheMapping::new("Stock", "stock::count", "i64"),
            ],
        }
    }
}

impl BlueprintNode for CheckStockNode {
    fn name(&self) -> &str {
        "CheckStockNode"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("ProductID", "String"),
            Pin::exec_out("Out"),
            Pin::data_out("Available", "bool"),
            Pin::data_out("Stock", "i64"),
        ]
    }

    fn pin_cache_mappings(&self) -> (Vec<PinCacheMapping>, Vec<PinCacheMapping>) {
        (self.input_mappings.clone(), self.output_mappings.clone())
    }
}
```

## 4.4 节点调用系统

节点内部可以直接调用无状态系统。不要把系统写成持状态对象。

```rust
pub async fn execute(
    &self,
    ctx: &mut ExecutionContext,
    inputs: HashMap<String, DataValue>,
) -> Result<NodeOutput> {
    let product_id: String = inputs
        .get("ProductID")
        .and_then(|v| serde_json::from_value(v.value.clone()).ok())
        .unwrap_or_default();

    let sys_ctx = ctx.inner().clone();
    let inventory = InventoryRepository;
    let product = inventory.execute(product_id, &sys_ctx).await?;

    let mut outputs = HashMap::new();
    outputs.insert("Available".into(), DataValue::new("bool", serde_json::json!(product.is_some())));
    outputs.insert("Stock".into(), DataValue::new("i64", serde_json::json!(0)));

    Ok(NodeOutput::Data(outputs))
}
```

如果能力是用 `#[define_operation]` 声明的，宏会同时生成系统和节点注册元数据，适合“AI 工具 + 工作流节点”共用同一个操作定义。

## 4.5 DynamicSystemNode

动态节点适合按名称调用 `SystemRegistry` 里的 `DynamicExecute`，例如 RPC runtime tool：

```rust
let executor = ctx.get_dynamic_system("PythonCtxProbe")?;
let mut input = HashMap::new();
input.insert("input".to_string(), serde_json::json!("--key rpc_demo_input"));

let output = executor.execute_dynamic(input, &ctx).await?;
```

这个路径让本地系统和远程工具可以共享一条执行抽象：`DynamicExecute`。

### 4.5.1 动态 RPC 节点的输出引脚

动态 RPC 节点的数据引脚来自 `RuntimeToolMetadata.outputs`。RPC 返回的
`AIOutput` envelope 不是节点 schema：Corework 展开 `AIOutput.result`，校验注册字段，
然后把每个字段写入对应输出引脚。工具声明 `page_id` 与 `url` 时，节点就暴露这两个
引脚，而不是虚构的 `Result` 引脚。

Chain 脚本直接引用声明字段：

```text
1: BrowserOpenPage --url $url
return page_id=1.page_id url=1.url
```

最终 `return` 会编译成 End 节点输出。只有 End 输出进入 workflow 的程序结果；中间
节点输出除非被显式 return，否则只在 workflow 内部存在。

## 4.6 节点注册

静态节点通过 `inventory` 收集：

```rust
pub static CHECK_STOCK_NODE_METADATA: NodeMetadata = NodeMetadata::new(
    "CheckStockNode",
    "1.0.0",
    "Inventory",
    "检查库存",
    "查询商品库存是否可用",
    &[
        PinMetadata { name: "In", kind: PinKind::ExecInput, data_type: "", description: "执行输入" },
        PinMetadata { name: "ProductID", kind: PinKind::DataInput, data_type: "String", description: "商品 ID" },
        PinMetadata { name: "Out", kind: PinKind::ExecOutput, data_type: "", description: "执行输出" },
        PinMetadata { name: "Available", kind: PinKind::DataOutput, data_type: "bool", description: "是否可用" },
    ],
    NodePermissions { bits: NodePermissions::NONE },
);

pub static CHECK_STOCK_NODE_FACTORY: NodeFactory = NodeFactory {
    metadata: &CHECK_STOCK_NODE_METADATA,
    constructor: || Box::new(CheckStockNode::default()),
};

inventory::submit!(CHECK_STOCK_NODE_FACTORY);
```

更推荐用 `#[define_operation]` 减少重复样板。

## 4.7 Chain Text

蓝图还支持给 AI 生成 / 修改的链式文本格式：

```text
start -> CheckStock -> Branch
Branch.True -> DeductStock -> CreateOrder -> end
Branch.False -> end
```

相关模块：

```text
chain_ast.rs
chain_compiler.rs
chain_compiler_v2.rs
chain_decompiler.rs
chain_id.rs
```

Chain Text 是编辑 / 生成语法，不改变核心执行模型。最终仍会编译为 blueprint JSON 并由 `BlueprintExecutor` 执行。

## 4.8 使用建议

- 节点只做编排胶水，复杂业务写进 `SystemOperation`。
- 节点输出写成明确的 cache key，避免多个节点覆盖同一个中间值。
- 对 AI 和 workflow 都要暴露的能力，用 `#[define_operation]`。
- 对运行时远程工具，用 `RuntimeToolMetadata + RpcStubSystem + register_dynamic()`。
- 不要把 workflow 描述成核心层级；它只是使用 corework 核心能力的一种编排方式。

下一篇：[05_执行单元_Module与StateMachine.md](05_执行单元_Module与StateMachine.md)
