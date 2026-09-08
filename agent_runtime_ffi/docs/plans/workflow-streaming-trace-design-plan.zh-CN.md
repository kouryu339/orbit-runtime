# Workflow 流式执行 Trace 设计与实施计划

> 状态：核心方案已实现；稳定对外契约以 `docs/zh/11-runtime-workflow-execution-contract.md` 为准。
>
> 范围：模板、Draft、Registered Workflow 与临时 Script 的真实执行过程。
> 本文保留设计依据和后续实施边界。

## 1. 目标与非目标

### 1.1 目标

Runtime 在工作流运行过程中产生结构化、按运行排序的执行事实，使宿主能够：

- 立即取得 `workflow_run_id`；
- 展示当前执行到哪个节点；
- 展示实际选择的 IF 分支和真实发生的循环迭代；
- 区分节点、审批、工具业务和工作流的状态；
- 持久化事件后根据 `sequence` 恢复页面；
- 从终态事件取得最终结果；
- 使用同一套协议处理模板、Draft、Registered Workflow 和临时 Script。

### 1.2 非目标

- Trace 不替代工作流输入、输出引脚，不参与节点间数据传递。
- Trace 不要求客户端根据静态 Blueprint 推断实际路径。
- 事件补读由 Host 持久化层负责，不承诺 Runtime 进程崩溃后的断点续跑。
- Runtime 不负责宿主的长期数据库持久化。
- Trace 不默认携带完整文件、超大对象或全部工具原始返回。

## 2. 当前实现基线

当前代码已经具备以下基础：

- 每次执行会生成 `run_id`；
- Trace 记录器能够产生节点 Started、Succeeded、Failed、Skipped 状态；
- 节点事件带有递增 `sequence`；
- 工作流执行入口会通过全局 workflow 事件线发布开始事件和节点事件；
- 节点结果已经有约 4 KiB 的 JSON 预览限制；
- RPC 工具层已经区分传输返回和 `error_code` 业务结果。

当前已经补齐 `execution_id`、父子执行作用域、统一 sequence、IF 与循环事件、业务失败语义、
两阶段执行通道和四种语言 SDK。同步执行接口继续兼容。尚未完成的是节点级审批等待状态机；
事件历史、页面恢复和补读明确属于 Host，而不是 Runtime 内存 Journal。

## 3. 总体架构

```text
模板 / Draft / Registered / 临时 Script
                    │
                    ▼
        workflow.start（只分配 run id）
                    │
          Host subscribe(run_id)
                    │
                    ▼
        workflow.run（统一运行入口）
                    │
                    ▼
      ExecutionContext + ExecutionScopeStack
          ├─ workflow identity
          ├─ node execution handle
          └─ branch / loop / iteration scope
                    │
                    ▼
       Workflow Executor / RPC / Approval
                    │ 产生真实执行事实
                    ▼
          Runtime 公共事件通道
                    │
                    ▼
          Host 持久化、补读与 UI
```

架构约束：

1. 公共事件是 Runtime 唯一 Trace 出口，Runtime 不保存历史副本。
2. `workflow.start` 只创建身份；Host 监听后才调用 `workflow.run`。
3. 所有执行入口复用同一个执行器和 Trace 逻辑。
4. Host 决定是否持久化，并负责历史查询和补读。

## 4. 执行身份模型

| 字段 | 含义 | 规则 |
|---|---|---|
| `workflow_id` | 工作流定义身份 | 临时 Script 允许缺失。 |
| `workflow_revision` | 本次执行使用的定义版本 | Draft/Registered 应提供；临时 Script 可提供内容摘要。 |
| `workflow_run_id` | 单次运行身份 | 接受启动请求时生成，运行期间不变。 |
| `node_id` | 静态 Blueprint 节点身份 | 在本次冻结的执行定义内解释。 |
| `execution_id` | 某节点本次实际调用身份 | 每次实际调用都生成，循环内重复节点不能复用。 |
| `parent_execution_id` | 实际父执行作用域 | 用于分支、循环迭代和嵌套控制结构。 |
| `sequence` | 运行内事件序号 | 由本次执行的 Trace Recorder 单点分配，从 1 严格递增。 |

兼容策略：

- 现有响应中的 `run_id` 暂时保留，值等同于 `workflow_run_id`；
- 新 Trace schema 统一使用 `workflow_run_id`；
- 临时 Script 不伪造 `workflow_id`，以字段缺失表达无持久定义；
- Agent 调用工作流时继续向工作流上下文传递可用的 `conversation_id`、`agent_id`、`turn_id`，但这些字段不是直接工作流执行的必需条件。

## 5. 运行与节点状态机

### 5.1 运行状态

```text
created → running → completed
                 ├→ failed
                 └→ cancelled（后续取消能力）
```

定义：

- `accepted`：已经分配运行 ID，执行任务尚未开始；
- `preparing`：加载、编译、校验输入和构建执行图；
- `running`：已经开始执行节点；
- `completed`：End/return 结果已经形成；
- `failed`：准备阶段或执行阶段失败；
- `cancelled`：预留的明确终态，不得映射为成功。

每次运行必须且只能进入一个终态。编译失败也必须通过同一个运行 ID 查询，而不是启动命令同步抛出后完全失去追踪身份。

### 5.2 节点状态

```text
started → waiting_approval → running → completed
   │            │              ├────→ failed
   │            └──────────────→ rejected/cancelled
   └───────────────────────────→ failed
```

对外可以保持较小的事件集合，但内部状态必须明确。审批批准只允许节点继续执行，不能直接代表节点成功。

### 5.3 终态约束

- 每个 `execution_id` 最多产生一个节点终态事件；
- `node.completed` 必须晚于该节点对应的工具、审批和控制体完成；
- `workflow.completed` 必须晚于 End 结果形成；
- 工具 transport 成功但业务错误时必须产生 `node.failed`；
- 节点失败后不得再产生该 `execution_id` 的 completed；
- 运行终态之后不得追加普通执行事件，只允许独立的审计修复事件版本。

## 6. 统一事件协议

基础信封：

```json
{
  "schema": "agent-runtime-workflow-trace/v1",
  "type": "node.completed",
  "workflow_id": "workflow-xxx",
  "workflow_revision": 7,
  "workflow_run_id": "wf-xxx",
  "sequence": 18,
  "timestamp": "2026-09-07T18:30:00Z",
  "node_id": "5",
  "execution_id": "exec-xxx",
  "parent_execution_id": "iteration-xxx"
}
```

事件集合：

### 6.1 工作流事件

- `workflow.started`
- `workflow.completed`
- `workflow.failed`
- 后续可加入 `workflow.cancelled`

`workflow.started` 提供有界、已脱敏的 `inputs_summary`。`workflow.completed` 的结果必须来自 End/return 输入引脚打包结果。

### 6.2 普通节点事件

- `node.started`
- `node.completed`
- `node.failed`
- 可选 `node.skipped`

完成事件可以包含：

- `tool_name`
- `display_name`
- 真实工具 `toai`
- `started_at`、`finished_at`、`duration_ms`
- `selected_output_pin`
- 有界 `inputs_summary` 和 `outputs_summary`

不应为了未选择分支中的每个静态节点批量制造 `node.skipped`。如需展示未走分支，应通过 `branch.selected` 的事实处理；否则复杂循环可能产生大量从未执行过的伪实例。

### 6.3 分支事件

- `branch.selected`

至少包含：

- IF 节点的 `node_id` 和 `execution_id`；
- `condition_value`；
- `selected_pin` 和 `selected_label`；
- 有界 `condition_summary`。

实现时需要明确分支作用域的结束边界。分支后的汇合节点恢复到上级作用域，不能永久成为 IF 的子节点。

### 6.4 循环事件

- `loop.started`
- `loop.iteration.started`
- `loop.iteration.completed`
- `loop.iteration.failed`
- `loop.completed`

循环本身和每次迭代都有独立执行身份：

```text
loop execution
  ├─ iteration execution 0
  │    ├─ node A execution
  │    └─ node B execution
  └─ iteration execution 1
       └─ node A execution
```

`loop.completed` 必须在所有实际迭代结束后产生。提前 `break` 时应包含 `completion_reason=break` 和真实 `iteration_count`，不能报告成遍历了全部输入。零项循环也要产生 started/completed，并报告 0 次。

### 6.5 审批事件

- `approval.requested`
- `approval.resolved`

通过 `tool_call_id` 与现有权限决议机制关联。resolved 至少区分 approved、rejected、cancelled、expired。拒绝或超时必须继续产生节点和运行的明确非成功结果。

## 7. 工具输出和数据引脚边界

RPC 工具可能返回 `error_code`、`to_ai` 和 `result` 信封。它的职责分别是：

- `error_code`：工具业务执行状态；
- `to_ai`：供 AI 或用户理解本次工具调用结果；
- `result`：用于将注册输出字段投影为节点输出；
- 注册输出引脚：工作流节点间唯一的数据连接契约；
- End/return 输入：工作流最终结果的唯一来源。

Trace 的 `outputs_summary` 从已经投影并校验的节点输出引脚生成，不要求客户端解析 RPC 信封。

需要区分真实 `toai` 和 Runtime 兜底生成的说明：

- 工具真实返回写入 `toai`；
- Runtime 合成说明写入单独的 `framework_summary`；
- 两者都需要在发布公共事件前脱敏和限长；
- 不允许用合成说明冒充工具真实输出。

节点成功条件至少包括：

1. 调用通道成功；
2. 工具响应协议有效；
3. `error_code` 表示成功；
4. 注册输出字段能够合法投影；
5. 后续审批或控制流没有返回非成功状态。

## 8. 启动、状态、补读和订阅

异步运行采用执行通道：

| 建议命令 | 作用 |
|---|---|
| `workflow.start` | 只创建一次性 `workflow_run_id`，不开始执行。 |
| `workflow.run` | 在宿主已经监听的 Run 通道上开始执行。 |

现有 `workflow.execute` 和 `workflow.execute_script` 保留同步语义。新宿主按
`start -> subscribe(run_id) -> run` 使用异步接口；Trace 持久化和补读由宿主负责。

### 8.1 启动响应

```json
{
  "schema": "agent-runtime-workflow-run-start/v1",
  "workflow_run_id": "wf-xxx",
  "status": "created"
}
```

它只表示 Runtime 创建了执行通道。只有后续 `workflow.run` 才会开始准备、编译和执行。

### 8.2 无首事件丢失窗口

推荐消费方式：

1. 宿主调用 `workflow.start` 获取 ID；
2. 建立只接收该 `workflow_run_id` 的监听；
3. 调用 `workflow.run`；
4. 按 `(workflow_run_id, sequence)` 排序、去重和持久化；
5. 页面恢复从 Host 自己的事件存储读取。

因为 `start` 不执行任何工作，所以监听建立前不会出现该 Run 的事件。

## 9. 资源与持久化边界

Runtime 仅为活动 Run 保存：

- `workflow_run_id`；
- `created/running` 控制状态；
- 后续取消、deadline 所需的控制句柄。

必须配置并测试以下上限：

- 单条 `toai` 字节数；
- 单个摘要字段字节数；
- 单条事件字节数；
- 活动运行并发上限；
- 终态结果事件最大字节数。

达到硬上限时必须生成明确的截断/溢出诊断，而不是无提示丢事件；终态投递后立即释放控制记录。

宿主持久化边界：

- Runtime 通过公共事件通道提供事实；
- Host 负责事件历史、补读接口和跨 Runtime 重启恢复；
- 事件历史恢复与工作流断点续跑是两个项目，不在本阶段混合实现。

## 10. 安全和摘要策略

所有内容必须在进入公共事件前完成过滤。

优先级：

1. 工作流输入和工具 schema 中的 `sensitive` 元数据；
2. 已知认证字段类别；
3. 名称启发式作为补充；
4. 类型专用摘要器。

默认策略：

- 密码、Token、Cookie、Authorization 等值替换为固定脱敏标记；
- 文件只输出名称、类型、大小、数量等允许元信息，不读取内容；
- 大数组提供总数和首尾有限预览；
- 大对象限制深度、键数和总字节数；
- 字符串截断必须标注 `truncated` 和原始长度；
- 不将完整 Trace 自动塞进 Agent 会话上下文；
- AI 需要详情时按 `workflow_run_id + execution_id` 定点读取。

`workflow.completed.result` 与事件大小上限存在天然冲突。当前边界是：

- End 结果在终态事件中内联，设置 16 MiB 硬上限；
- 更大的业务数据应由工具写入宿主持久存储，让 End 返回稳定引用；
- Runtime 不保存结果正文，也不生成指向自身内存的 `result_ref`。

## 11. 输入参数元数据

输入表单能力应从统一工作流输入契约导出，而不是客户端解析 Script 文本。每项至少提供：

- `name`
- `display_name`
- `type`
- `required`
- `default`
- `description`
- `sensitive`
- `element_type`
- 可选的 `format`

建议先将 File 表达为 `type=String, format=file`，Array<File> 表达为 `type=Array, element_type=String, element_format=file`。这样可以支持文件选择 UI，又不在本阶段悄悄扩大工作流基础类型系统。宿主仍负责把文件选择结果转换为 Runtime 可访问的句柄或路径。

模板、Draft、Registered 和 Script/Blueprint 往返必须保留这些元数据。

## 12. 代码改造范围

计划涉及的主要模块：

1. `corework/src/workflow/execution/trace.rs`
   - 将最终 Trace 收集器升级为执行事件定义和记录入口；
   - 引入 execution handle，不再按节点名称回填；
   - 统一摘要、限长、脱敏边界。

2. `corework/src/workflow/execution/execution_context.rs`
   - 保存当前执行作用域栈；
   - 创建和传播节点、分支、循环、迭代身份；
   - 保持 Agent 到 Workflow 到 RPC 的上下文传播。

3. `corework/src/workflow/execution/executor.rs`
   - 调整节点终态时机；
   - 在真实控制流选择位置产生 branch/loop 事件；
   - 正确关闭 break、失败和嵌套作用域。

4. `corework/src/workflow/workflows/executor.rs`
   - 所有执行入口统一创建执行上下文；
   - 发布完整工作流生命周期；
   - 移除每次调用临时创建无界转发队列的依赖。

5. Runtime FFI
   - 新增 `workflow.start` / `workflow.run` 命令；
   - 在 capabilities 中声明新命令、事件和 schema；
   - 旧同步命令保持兼容。

6. Runtime SDK
   - 提供两阶段异步运行和按 Run ID 监听；
   - 提供 SDK 级过滤与去重辅助；
   - 不隐藏终态错误。

7. 文档
   - Trace 事件契约；
   - Host 持久化建议；
   - 输入参数元数据；
   - 旧同步接口迁移示例。

## 13. 分阶段实施计划

### 阶段 A：协议和内部身份

- 冻结 v1 事件字段和终态规则；
- 引入 `execution_id`、作用域栈和统一 sequence；
- 保持旧同步响应不变；
- 增加顺序节点、失败节点、循环重复节点的内部测试。

完成门槛：同一静态节点多次执行时，每次拥有不同 execution_id，开始和终态能准确配对。

### 阶段 B：控制流和工具语义

- 补齐 IF、循环、迭代和嵌套事件；
- 修正循环节点完成时机；
- 接入真实 `toai`、业务错误和审批状态；
- 明确 break、拒绝和异常终态。

完成门槛：客户端完全依靠事件即可复现实际执行树，不需要从 Blueprint 猜路径。

### 阶段 C：执行通道和宿主持久化

- 新增两阶段 `workflow.start` / `workflow.run`；
- SDK 按 `workflow_run_id` 订阅公共事件；
- Runtime 仅保留活动 Run 控制记录，终态后释放；
- Host 持久化事件并自行提供分页补读和单步详情。

完成门槛：宿主先获得 ID 并监听，再开始执行；运行中能够持续收到事件且 Runtime 不保存历史 Trace。

### 阶段 D：SDK、宿主契约和输入表单

- SDK 暴露两阶段运行与按 Run ID 监听 API；
- 补齐 capabilities 和中英文契约；
- 导出输入参数元数据；
- 编写 Host 持久化和 UI 消费示例。

完成门槛：至少一个 SDK 和测试宿主完成真实流式渲染与重连补读，其他 SDK 类型定义保持协议一致。

### 阶段 E：兼容、压力与发布验收

- 验证旧 `workflow.execute` 行为；
- 验证高频迭代、慢消费者、超大输出和历史淘汰；
- 验证不同执行入口使用同一事件协议；
- 更新发布说明和版本能力矩阵。

## 14. 测试与验收矩阵

| 场景 | 必须验证 |
|---|---|
| 普通顺序 | 运行过程中已收到 started/completed，顺序严格递增。 |
| IF True/False | 明确报告实际 pin，不靠后续节点倒推。 |
| 零次循环 | 有 loop started/completed，iteration_count=0。 |
| 多次循环 | 同一 node_id 产生不同 execution_id。 |
| break | 真实次数和 completion_reason 正确。 |
| 嵌套循环 | parent_execution_id 构成正确执行树。 |
| 工具业务失败 | RPC transport 成功也产生 node/workflow failed。 |
| 输出投影失败 | 不产生成功节点，不暴露虚构输出。 |
| 审批批准 | requested、resolved、执行、completed 顺序正确。 |
| 审批拒绝/超时 | 不报告成功，状态可诊断。 |
| 编译失败 | 已接受运行拥有 run_id、阶段和结构化诊断。 |
| 页面切换 | Host 从自己的持久化记录按 sequence 恢复。 |
| 订阅竞争 | start 后先监听对应 run_id，再调用 run，不丢首事件。 |
| 重复通知 | 按 run_id + sequence 去重。 |
| 慢消费者 | 不导致 Runtime 无界内存增长。 |
| End 结果 | 权威最终结果与同步执行返回一致。 |
| 大结果 | 超过事件硬上限明确失败，业务应返回外部持久化引用。 |
| 敏感输入 | 公共事件不泄露。 |
| 四类入口 | 模板、Draft、Registered、临时 Script 事件语义一致。 |

测试不能只在执行结束后检查最终 JSON。至少应使用阻塞测试工具或可控 barrier，证明第一个节点尚未结束时宿主已经能够读到前置事件。

## 15. 风险与待确认决策

### 推荐直接采用的决定

- Runtime 仅发布事件，Host 负责持久化与补读；
- start 创建通道、宿主监听、run 执行，消除首事件竞争；
- 旧同步接口兼容，新异步接口提供完整能力；
- 临时 Script 的 `workflow_id` 可空；
- File 先作为输入格式元数据，不新增基础执行类型；
- 大结果由工具保存后返回外部引用，不由 Runtime 暂存；
- 本阶段不包含崩溃后工作流断点续跑。

### 实现前需冻结的参数

- workflow revision 在 Draft、Registered、模板中的统一表达；
- 是否在 v1 提供取消，或仅预留 cancelled 状态；
- 是否为分支本身建立独立 scope execution，还是直接以 IF node execution 作为父级；
- 各 SDK 首批同步上线范围。

## 16. Engineering Review

**Architecture：** 沿用工作流执行器和 Runtime 事件线，以两阶段执行通道分离身份创建和实际运行；数据引脚、执行事实和宿主持久化职责分离。

**Code：** 首先替换按节点名称关联 Trace 的机制，再扩展事件；避免在各执行入口重复实现。

**Concurrency：** sequence 只保证单次运行的事件记录顺序；execution_id 和 parent_execution_id 表达执行关系。通知可重复，消费必须幂等。

**Resource：** 禁止无界运行历史、无界订阅队列和默认完整大对象事件；额度必须配置化并可观测。

**Audit：** 记录实际分支、迭代、审批、工具业务错误和运行阶段；不伪造未执行节点。

**Verification：** 以运行中事件、先监听后运行、父子关系和失败语义的端到端测试作为发布门槛，不能只依赖编译通过。

**Risks：** 控制流作用域关闭错误、旧接口兼容、敏感数据摘要、大结果一致性和慢消费者内存压力。

**Decisions Needed：** 本文第 15 节的参数需要在编码前冻结，其余架构可以按阶段推进。
