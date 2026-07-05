# AI Assistant 系统结构与机制

本目录解释 `ai-assistant/src` 内部 AI 系统如何工作：conversation state、Agent
协作、工具执行、Skills、prompt、持久化和运行时机制。

产品接入请先看 `examples/guides`。ABI 与 JSON 契约看 `agent_runtime_ffi/docs`。
SDK 包使用看 `sdk`。

## 阅读顺序

1. [架构设计](01_architecture.md)

   Conversation、ConversationState、Gateway、Cluster 和 Agent 的职责边界。

2. [状态机与对话引擎](02_state_machine_and_conversation_engine.md)

   执行循环、状态流转、命令准入和 conversation 推进方式。

3. [EXEC 与工具执行](03_exec_line_protocol_and_tool_execution.md)

   模型工具协议、工具白名单、执行结果和 `to_ai`。

4. [Skill 与提示词](04_skill_system_and_prompts.md)

   role/feature/system Skills、渐进加载和 prompt 组装。

5. [Agent 与持久化](05_agent_and_persistence.md)

   focus 接力、后台任务、ledger、snapshot 和恢复状态。

6. [运行时机制](06_runtime_mechanics.md)

   检索、压缩、暂停/关闭、事件和 shutdown 机制。

## 边界

这里讲机制，不定义公共宿主契约、不承担 SDK 接入顺序说明，也不写产品级集成策略。
