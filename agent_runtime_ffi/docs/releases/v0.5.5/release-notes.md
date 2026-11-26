# Orbit Runtime v0.5.5

- **JSON 工具调用**。文本模式默认使用 `{"tool":"ToolName","args":{}}`，兼容旧 `EXEC`。原生函数调用和文本模式共用参数校验与授权，RPC 通过 `args_json` 接收带类型的参数。
- **异步工作流**。`workflow.execute` 返回 `run_id` 和 `status: "accepted"`。宿主先监听，再按 `payload.run_id` 匹配 `workflow.execution_completed` 事件。旧的 `workflow.start` 和 `workflow.run` 已移除。正在执行的工作流及其引用继续受更新保护。
- **会话链接交由宿主处理**。Lit 和 Svelte 使用 `capabilities.openLink` 或 `agent-conversation-link-open` 交给宿主决定打开方式。

宿主升级时，请将同步读取执行结果的代码改为读取终态事件。事件可能先于提交响应到达，监听端应先暂存，再按 `run_id` 关联。成功读取 `payload.result`，失败读取 `payload.error`。

发布产物包含 Windows DLL、Linux SO、浏览器 WASM 和 JS/TS SDK、独立 Svelte JS 与 SDK 源码包。原生 ABI 版本为 1.3。
