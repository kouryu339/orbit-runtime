# 2 02 状态机与对话引擎

所有 Agent 使用同一套四状态状态机：

```text
suspended -> thinking -> executing -> thinking
                    \-> saying -> suspended
```

默认/持久 Agent 从 `suspended` 开始；已注入任务的临时 Agent 从 `thinking` 开始。

## 2.1 状态职责

| 状态 | 职责 |
|---|---|
| `suspended` | 等待用户输入、任命、报告或恢复事件。 |
| `thinking` | 构建 system prompt/history，执行默认检索，调用 LLM，解析正文决策。 |
| `executing` | 校验 ACTIVE_TOOLS，执行一个或多个 local/RPC 工具并回灌 `to_ai`。 |
| `saying` | 将最终 assistant 内容写入 ledger，发布前端状态并结束 turn。 |

## 2.2 命令准入

宿主命令先经过 conversation command gate。`send_message`、`pause`、summary model 和
compact 等命令产生 admission 结果；同一 conversation 的冲突操作不会无序修改状态。
关闭流程取得 close guard，后续命令被拒绝。

`send_message` 的同步部分只负责写入用户消息、推动状态机并启动 driver；LLM 执行在
driver task 中继续。宿主通过事件观察进度和最终结果。

## 2.3 Pause

pause 设置取消/暂停状态并请求 thinking 收敛。LLM 请求可取消；已经进入的工具调用
通常不会被强制中断，而是在工具边界后进入 `suspended`。暂停会清理 pending tools、
display commands 和 next-state 等短期字段。

## 2.4 Thinking 回合

`MAX_THINKING_ROUNDS=0` 表示使用内部默认策略，不应解释成无限循环。每轮 thinking
都会重新评估工具结果、plan、ledger history、动态上下文和 stop reason。
`ContinueThinking` 只请求额外一轮推理，不执行业务动作。

## 2.5 结束条件

Agent driver 在状态回到 `suspended`、任务达到终态或发生不可恢复错误时结束。
后台任务必须通过 `ReportAgentTask` 发布 completed/failed/canceled 报告；Runtime 再将
报告写入委托方 ledger并完成任务，不依赖前台 Agent 轮询榜单。

下一篇：[03 EXEC 行式协议与工具执行](03_exec_line_protocol_and_tool_execution.md)
