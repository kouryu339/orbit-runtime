# AI Assistant System Architecture

These documents describe the internal AI system implemented by
`ai-assistant/src`. Read them when you need to understand how the Runtime builds
Agent context, drives state machines, executes tools, handles Skills, and emits
conversation state.

For product onboarding, start with `examples/guides`. For ABI and JSON contract
details, use `agent_runtime_ffi/docs`. For package APIs, use `sdk`.

## Reading Order

1. [Architecture](01_architecture.md)

   Conversation, ConversationState, Gateway, Cluster, and Agent responsibilities.

2. [State Machine And Conversation Engine](02_state_machine_and_conversation_engine.md)

   Execution loop, runtime states, command admission, and conversation flow.

3. [EXEC Line Protocol And Tool Execution](03_exec_line_protocol_and_tool_execution.md)

   Model tool protocol, allowlists, execution results, and `to_ai`.

4. [Skill System And Prompts](04_skill_system_and_prompts.md)

   Role/feature/system Skills, progressive loading, and prompt assembly.

5. [Agent And Persistence](05_agent_and_persistence.md)

   Focus handoff, background tasks, ledger, snapshots, and recovery state.

6. [Runtime Mechanics](06_runtime_mechanics.md)

   Retrieval, compaction, pause/close, events, and shutdown mechanics.

## Boundary

These documents are mechanism explanations. They should not define public host
contracts, SDK onboarding order, or product-specific integration policy.
