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
- Background tasks: `CreateBackgroundAgentTask` and `ReportAgentTask`.
- Per-agent retrieval: `RagRetrieve` when retrieval is configured.

These 16 names are the complete current set of local AI operations that a
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

Workflow Editor has an isolated allowlist: `openWorkflowDraft`, `readWorkflow`,
`updateCurrentWorkflowDraft`, `registerCurrentWorkflowDraft`,
`compileWorkflowScript`, `testWorkflow`, and `searchSkillRefs`. These tools
select and mutate the same Draft/Registered catalog exposed by the Runtime ABI;
the browser canvas is a view, not a second draft store.

Agent Test Studio has two isolated roles. The supervisor receives
`AdversaryCreate`, `AdversaryDestroy`, `AdversaryInspect`, and `WriteMarkdown`;
the adversary receives only `AdversaryConclude`. The supervisor explicitly
forbids `Wait`, polling, and planning tools. Studio tools also require the
corresponding Studio runtime/context to be active.

Internal prompt, ledger, `Draft*`, and workflow-node systems are not a default
business-agent tool set merely because they are locally registered.

For a background master-worker model, grant `CreateBackgroundAgentTask` to the
front agent and `ReportAgentTask` to the worker profile. Register the worker as
a resource profile, but only predeclare the front agent in the cluster. Runtime
creates a unique background instance for each task; it does not take focus and
must finish by reporting to the task board and delegator ledger.

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
