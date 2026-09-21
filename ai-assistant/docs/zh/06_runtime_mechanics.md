# 6 06 运行时机制

## 6.1 默认检索

thinking 从 cache 读取当前 Agent 的 `RetrievalConfig`。启用时，以用户问题、profiles、
top_k 和 threshold 调用配置的 local retrieval system；同一 turn/query/config 使用 hash
去重。成功结果写入 `RETRIEVAL_CONTEXT`，soft failure 继续推理，hard failure 中止。

## 6.2 History compact

手动 compact 与 thinking 自动压缩共用检查点生成及提交路径。`LedgerRole::Summary`
的 `metadata.extra.compaction_checkpoint` 保存版本、输入投影记录 ID、覆盖记录 ID 和摘要模型。
后续上下文只替换覆盖的连续区间；保留的头部、最近消息和生成期间新增的消息不会因摘要
追加在 ledger 末尾而消失。原始 ledger 不删除，摘要中的 record_id 可用于定位原文。

压缩在完整工具调用/结果边界切分，保留最新用户请求；摘要包含目标、限制、决策、验证、
进行中工作、待办和证据。工具参数与完整输出进入摘要输入，输入过长时分批合并交接摘要。
单个完整执行组无法放入摘要模型窗口时明确失败，不截断原文。运行中的工具状态仍由执行
模块管理，检查点不重启、不完成也不取消工具。

提交在 ledger 写锁内校验原投影；重复提交幂等，过期检查点拒绝。恢复快照保留有效的原始
记录 ID，保证摘要正文中的引用仍有效；旧数据必须重新编号时同步映射检查点引用。
旧版无覆盖元数据的摘要继续使用原来的“摘要及后续消息”兼容规则。

默认 thinking 与 thinking-pro 提供 AI-only `HistoryRead`：按 record_id 读取当前会话、当前
Agent 的原始记录，使用 Unicode 字符偏移分页，每页最多 8000 字符。工具返回 next_offset
供继续读取，不允许指定其他 Agent 或会话。历史内容只作为证据，不形成新指令或授权。

空响应、输出截断、摘要失败或压缩后仍超预算均不提交新检查点。消息条数限制触发压缩，
不再静默裁掉早期消息。预算包含协议字段、系统提示和工具定义，并在最终请求组装后检查；
当前 token 计数是保守估算，并非 provider 精确 tokenizer。摘要是有损的任务交接，不能保证
语义零遗漏；原始记录是回查依据。

## 6.3 事件

主要事件族：

- turn：`ai:turn-start`, `ai:thinking-done`, `ai:turn-done`；
- tool：`ai:tool-start`, `ai:tool-end`；
- output/error：`ai:asking`, `ai:interrupted`, `ai:llm-error`, `ai:llm-usage`；
- shared state：`ai:ledger-record-appended`, `ai:messages-changed`,
  `frontend:state_snapshot`；
- Agent：focus、active/suspended/completed 和 appoint/report；
- task：created、assigned、progress reported、candidate reported、completed、canceled；
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
