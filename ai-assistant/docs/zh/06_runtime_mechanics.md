# 6 06 运行时机制

## 6.1 默认检索

thinking 从 cache 读取当前 Agent 的 `RetrievalConfig`。启用时，以用户问题、profiles、
top_k 和 threshold 调用配置的 local retrieval system；同一 turn/query/config 使用 hash
去重。成功结果写入 `RETRIEVAL_CONTEXT`，soft failure 继续推理，hard failure 中止。

## 6.2 History compact

compact 是显式 conversation 命令。Gateway 对目标 Agent 加命令门，使用 summary model
生成摘要，写入 `LedgerRole::Summary` 并发布 compact done/skipped/failed。summary 不在
普通聊天中渲染，但成为后续 LLM history 的起点。

## 6.3 事件

主要事件族：

- turn：`ai:turn-start`, `ai:thinking-done`, `ai:turn-done`；
- tool：`ai:tool-start`, `ai:tool-end`；
- output/error：`ai:asking`, `ai:interrupted`, `ai:llm-error`, `ai:llm-usage`；
- shared state：`ai:ledger-record-appended`, `ai:messages-changed`,
  `frontend:state_snapshot`；
- Agent：focus、active/suspended/completed 和 appoint/report；
- task：created、assigned、reported、completed；
- plan 与 Workflow Studio draft update。

用户可见事件会补充 conversation/agent/turn 元数据。FFI 再包装成
`agent-runtime-event/v1`，由宿主通过 pull queue 获取。

## 6.4 错误与停止

LLM 空响应和可重试错误按 thinking 策略重试；fatal error 发布结构化 `LLM_ERROR`。
pause 通过 cancellation token 终止可取消的 LLM 请求，工具调用在边界收敛。每轮都会
记录 stop reason、thinking round 和 pending 状态，防止无界自动继续。

## 6.5 Plan 与 Wait

Plan 工具维护独立 `CURRENT_PLAN`，不会写入宿主动态快照。`Wait` 等待 timeout 或当前
scope/conversation 的指定事件；它只负责让出执行，不读取任务结果，也不应用短周期
循环模拟轮询。角色是否能用 Wait 由 Skill 白名单决定。

## 6.6 关闭

Conversation shutdown 与 Runtime shutdown 是两层。前者停止 conversation drivers；
后者还冻结整个 handle、等待在途 FFI 调用、关闭所有 Runtime 服务与事件生产者。
只有 ABI `agent_runtime_shutdown_v1` 返回 OK 后，宿主才可 destroy handle。
