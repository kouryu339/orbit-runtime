# 更新日志

## 2026-05-04

### 文档

- 重写 `01` 到 `08` 文档，按当前代码更新架构叙述。
- 明确 corework 核心是无状态宏定义系统、`Cache` / `ScopedCache` 和 `ExecutionUnit`，不是三层架构。
- 明确 `workflow`、`StateMachine`、`Saga`、`Module` 是上层能力编排封装。
- 明确 `RAG` 目前只有类型和 trait 骨架，尚未实现可用检索链路。
- 清理 DLL/RPC 阶段性完成记录；当前 RPC 工具协议以 `09_RPC工具协议v1.md` 和 `proto/corework_agent_tool_v1.proto` 为准。

## v0.2.0 (2026-01-16) - 工作流系统重构

### ⚠️ 重大变更

#### 删除的模块
- **Sequential Workflow**：删除了基于 Step/StepResult 的顺序工作流系统
- **DAG Workflow**：删除了 DAG（有向无环图）工作流系统
- **Common Workflow 工具**：删除了共享的工作流辅助函数

#### 保留的核心模块
- ✅ **Blueprint Workflow**：唯一保留的工作流实现（类似 UE 蓝图）
- ✅ **Pipeline**：数据处理管道（泛型 Stage<T>）
- ✅ **StateMachine**：有限状态机
- ✅ **Saga**：长事务协调与补偿
- ✅ **Cache**：缓存系统（含 TTL 和作用域缓存）
- ✅ **Event**：事件总线
- ✅ **System**：业务逻辑系统抽象
- ✅ **World**：全局资源容器
- ✅ **Context**：执行上下文

### 📦 新增功能

#### Blueprint Workflow 增强
```rust
// 支持的节点类型
- EntryNode: 入口节点
- TaskNode: 任务节点（调用 System）
- BranchNode: 分支节点（IF 条件）
- SequenceNode: 顺序执行节点（Then0, Then1...）
- ForLoopNode: 循环节点
- PureFunctionNode: 纯函数节点（无副作用）

// Pin 系统
- 输入/输出引脚
- 类型检查（Exec, Int, Float, String, Bool, Object）
- 节点连接验证
```

#### Cache 系统增强
- ✅ TTL（生存时间）支持
- ✅ 惰性删除策略（在 get/exists 时自动清理过期项）
- ✅ 作用域缓存（ScopedCache）
- ✅ 自动清理（Drop 时异步清理）
- ✅ 批量操作支持 TTL
- ✅ 动态设置过期时间（expire 方法）

#### Pipeline 增强
- ✅ 泛型 Stage<T> trait
- ✅ 支持任意数据类型的链式处理
- ✅ 异步执行
- ✅ Context 参数传递

#### StateMachine 增强
- ✅ 类型安全的状态定义
- ✅ 事件驱动的状态转换
- ✅ 状态变更监听器
- ✅ 非法转换检测

### 🔧 API 变更

#### 导入路径变更
```rust
// 旧 API（已删除）
use corework::workflow::{Step, StepResult, WorkflowBuilder};
use corework::workflow::sequential::*;
use corework::workflow::dag::*;

// 新 API
use corework::workflow::blueprint::*;
use corework::pipeline::{Pipeline, Stage};
use corework::state_machine::StateMachine;
```

#### Context 路径变更
```rust
// 旧
use corework::instance::Context;

// 新
use corework::orchestration::Context;
```

### 📚 文档更新

#### 更新的文档
- ✅ `00_PROJECT_OVERVIEW.md`：更新模块表和快速判断指南
- ✅ `01_ARCHITECTURE.md`：更新架构设计，替换为 Blueprint
- ✅ `02_USER_GUIDE.md`：更新使用指南，提供 Blueprint 示例
- ✅ `03_CORE_MODULES.md`：更新 API 参考，添加 Blueprint API
- ✅ `CHANGELOG.md`：新增更新日志（本文件）

### 🧪 新增测试

#### 测试文件
- ✅ `examples/basic_test.rs`：核心功能测试（Cache, Event, DataType, World, Context）
- ✅ `examples/pipeline_statemachine_test.rs`：Pipeline 和状态机测试
  - 数字处理管道
  - 字符串处理管道
  - 订单状态流转
  - 交通灯状态机
  - 多级审批流程
- ✅ `examples/cache_cleanup_test.rs`：缓存清理测试
  - TTL 过期自动清理
  - exists 方法过期检查
  - 批量操作 TTL
  - 动态设置过期时间
  - 计数器 TTL
  - flush 清空所有缓存
  - 自定义默认 TTL
- ✅ `examples/scoped_cache_test.rs`：作用域缓存测试
  - 作用域隔离
  - 自动清理（Drop 时）
  - 手动清理
  - flush 只清理当前作用域
  - 批量操作作用域隔离
  - 计数器作用域隔离
  - 多租户场景

### 📊 测试覆盖率

所有测试均已通过：
```
✅ basic_test.rs - 5/5 通过
✅ pipeline_statemachine_test.rs - 5/5 通过
✅ cache_cleanup_test.rs - 7/7 通过
✅ scoped_cache_test.rs - 8/8 通过
```

### 🎯 迁移指南

#### 从 Sequential Workflow 迁移到 Blueprint

**旧代码（Sequential Workflow）**：
```rust
use corework::workflow::{Step, StepResult, WorkflowBuilder};

let workflow = WorkflowBuilder::new("my_workflow")
    .add_step(Step1)
    .add_step(Step2)
    .build();

for step in workflow.steps() {
    step.execute(&ctx).await?;
}
```

**新代码（Blueprint Workflow）**：
```rust
use corework::workflow::blueprint::*;

let mut workflow = BlueprintWorkflow::new("my_workflow");

workflow.add_node("entry".into(), BlueprintNode::Entry(EntryNode::new()));
workflow.add_node("task1".into(), BlueprintNode::Task(TaskNode::new("任务1", "sys", "op")));
workflow.add_node("task2".into(), BlueprintNode::Task(TaskNode::new("任务2", "sys", "op")));

workflow.connect("entry".into(), "exec_out".into(), "task1".into(), "exec_in".into());
workflow.connect("task1".into(), "exec_out".into(), "task2".into(), "exec_in".into());

let executor = BlueprintExecutor::new(workflow);
executor.execute(&ctx).await?;
```

#### Pipeline 使用（数据转换场景）

如果你的工作流只是数据转换链（无副作用），建议使用 Pipeline：

```rust
use corework::pipeline::{Pipeline, Stage};

#[derive(Debug, Clone)]
struct ProcessStage;

#[async_trait]
impl Stage<String> for ProcessStage {
    async fn process(&self, input: String, ctx: &Context) -> Result<String> {
        Ok(input.to_uppercase())
    }
    
    fn name(&self) -> &str { "process" }
}

let pipeline = Pipeline::new("data_pipeline")
    .add_stage(ProcessStage);

let result = pipeline.execute("hello".to_string(), &ctx).await?;
```

### 🚀 性能改进

- **缓存系统**：惰性删除策略，减少定时器开销
- **作用域缓存**：Drop 时异步清理，不阻塞主线程
- **Blueprint**：Pin 连接系统，支持复杂流程控制

### 🐛 已修复的问题

- 修复了 `anyhow::Error` 到 `FrameworkError` 的转换问题
- 修复了 Context 导入路径不一致的问题
- 修复了作用域缓存的追踪 key 重复问题
- 修复了 Pipeline 泛型约束不完整的问题

### 📝 待办事项

- [ ] Blueprint 可视化编辑器（未来版本）
- [ ] Blueprint 序列化/反序列化（保存蓝图到文件）
- [ ] Blueprint 调试工具（断点、步进）
- [ ] 主动清理策略（定时清理过期缓存）
- [ ] 分布式缓存支持（Redis、Memcached）
- [ ] Saga 可视化监控
- [ ] 性能基准测试

### 🤝 贡献

感谢所有贡献者！

### 📄 许可证

MIT License

---

**完整测试命令**：
```bash
# 核心功能测试
cargo run --example basic_test --quiet

# Pipeline 和状态机测试
cargo run --example pipeline_statemachine_test --quiet

# 缓存清理测试
cargo run --example cache_cleanup_test --quiet

# 作用域缓存测试
cargo run --example scoped_cache_test --quiet

# 编译整个项目
cargo build --release
```
