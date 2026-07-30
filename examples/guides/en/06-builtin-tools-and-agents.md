# 6 Configure Built-in Tools and Multi-Agent Collaboration

Runtime provides collaboration tools, but role Skills must explicitly allow
them. Multi-agent behavior is composed from agent profiles, cluster instances,
focus, and tool allowlists.

Local registration does not make a tool globally visible. The public local
groups are:

- Utility: `Wait`, `ContinueThinking`, and `WriteMarkdown`.
- Progressive Skills: `GetSkillsList` and `UpdateSkills`.
- Planning: `PlanWrite`, `PlanUpdate`, and `PlanFinish`.
- Collaboration: `CreateAgent`, `AppointAgent`, `DismissAgent`, `ListAgents`,
  and `ReportToAgent`.
- Background tasks: `CreateBackgroundAgentTask`, `WaitAgentTask`,
  `PauseAgent`, `RequestAgentTaskInput`, `RespondAgentTaskInput`,
  `UpdateAgentTask`, `ReportAgentTaskProgress`, `ReportAgentTask`,
  `CompleteAgentTask`, and `CancelAgentTask`.
- Per-agent retrieval: `RagRetrieve` when retrieval is configured.

These 24 names are the complete current set of local AI operations that a
normal Agent Skill may reference. Ledger, prompt-building, Skill-loading, and
Draft systems also run in the local registry, but they are state-machine
internals rather than normal Agent tool contracts.

A tool description is exposed to the AI only when an active system, role, or
feature Skill explicitly references that tool in `tools`. Registration alone
does not advertise the tool. Use one rule for every tool: include it through a
Skill when the Agent needs it, and leave it out otherwise.

Keep each role Skill to the minimum tool set required by that role's core
responsibility. Put composable business capabilities into feature Skills and
activate them as needed. Runtime deduplicates tools by name when multiple active
Skills reference the same tool, so modular Skills do not duplicate tool
descriptions or add repeated context cost.

The system thinking Skill follows the same rule: its progressive-Skill,
planning, and `ContinueThinking` tools are present because its own `tools`
field explicitly references them, not because they bypass the allowlist.
`Wait` yields until a timeout or scoped event; it should replace polling, but
does not itself read a task result.

Workflow Editor uses the unified `listWorkflows`, `readWorkflow`,
`createWorkflowDraft`, `updateWorkflow`, `compileWorkflow`, `testWorkflow`,
`registerWorkflow`, `deleteWorkflow`, `executeWorkflow`, and
`executeWorkflowScript` tools through its `workflow_editor` role. Studio-only
`searchSkillRefs` searches design references. The built-in editor also selects
`thinking-pro` for common script syntax and temporary multiline execution.
Ordinary Agents selecting the same thinking replacement gain only
`executeWorkflowScript`; a host role/feature Skill must separately grant
persistent catalog tools. All callers share the Runtime ABI Draft/Registered
catalog; the browser canvas is only a view.

Agent Test Studio has two isolated roles. The supervisor receives
`AdversaryCreate`, `AdversaryDestroy`, `AdversaryInspect`, and `WriteMarkdown`;
the adversary receives only `AdversaryConclude`. The supervisor explicitly
forbids `Wait`, polling, and planning tools. Studio tools also require the
corresponding Studio runtime/context to be active.

The default `thinking` Skill stays lightweight. Legacy Draft systems and
file-path Workflow execution tools are removed rather than exposed as another
business-Agent tool set.

For a background master-worker model, grant `CreateBackgroundAgentTask` to the
front agent and reporting tools to the worker profile. Once a task is created,
Runtime adds authorization-checked wait, response, update, complete, cancel, and
pause tools to the delegator, plus input-request, progress-report, and candidate
final-report tools to the worker. It also projects the task id plus exact
assignee id into the delegator's dynamic tail snapshot. `WaitAgentTask` waits by task id and returns early when
any task delegated by that Agent requests input or submits a candidate result,
when the waited task becomes terminal, on new user input, or on timeout.
Unrelated task changes still do not wake it. An attention event reports its task separately, so the original
wait target is not mistaken for completed work.
The delegator answers by `task_id + request_id`; Runtime resolves the current
assignee and injects the answer without accepting a target Agent id. When a
worker requests input, an internal Corework System validates the caller's
conversation, ExecutionUnit, and Agent identity, derives the delegator from the
task relation, and wakes it only when stopped. This System is not exposed as an
AI tool. User input and internal wakeups share one per-Agent dispatch gate and
one driver slot.
The generic `Wait` tool remains responsible for ordinary timeout and event
waiting rather than task completion. However, a pending input request or candidate
result from a directly delegated task ends it early with `wake_reason=external_attention` and
an `attention_task_id`; the result explicitly says that the original wait
condition has not completed.

When a
parent goal changes, call `UpdateAgentTask` once for each affected task. This
increments that task's revision and injects the update into its current assignee.
`PauseAgent` supports `wait_for_tool` and `detach_tool`; the latter reports
`interrupted_unknown` and must not be retried before external verification.
Register the worker as
a resource profile, but only predeclare the front agent in the cluster. Runtime
creates a unique background instance for each task; it does not take focus and
submits candidate completion through the task board and delegator ledger. Only
the delegator accepts, revises, or cancels that task.

For persistent focus handoff, predeclare all agents in the cluster and set
`focus_agent_id`. Grant `AppointAgent` to coordinating roles and
`ReportToAgent` to specialists. `AppointAgent` transfers responsibility and
focus; `ReportToAgent` may return focus through its handoff option. Persistent
focus collaboration has no side channel for making a non-focus Agent think in
parallel; use background tasks for actual concurrency.

Treat focus collaboration primarily as a context-isolation optimization. It is
well suited to weakly related tasks with clear responsibility boundaries,
because each role loads only its own Skills and history. Prefer one Agent for
strongly related steps that depend on continuous dialogue or implicit prior
reasoning: a stable conversation is generally more reliable than uncertain
multi-Agent handoffs. Use focus handoff for correlated work only when the Skill
context is large enough that isolation clearly outweighs the transfer cost.

Use a concrete instance id for focus when one profile has multiple instances.
A profile id is accepted only when it resolves unambiguously.

Other built-ins include `CreateAgent` for a direct OneShot agent,
`DismissAgent`, and diagnostic `ListAgents`. Do not grant every collaboration
tool to every role. Authority to delegate, report, dismiss, or move focus is a
role policy expressed through `SKILL.md.tools`.

See the [Chinese guide](../zh/06-builtin-tools-and-agents.md) for complete resource,
cluster, and Skill examples.
