# 4 Skill System and Prompts

Skills provide runtime instructions for Agent identity and capabilities.

## 4.1 Skill Types

- **Role skill**: defines the Agent role, operating style, and responsibility
  boundary.
- **Feature skill**: adds optional capabilities such as workflow, file
  operations, web fetch, Word, PPTX, or Excel.
- **System skill**: built-in runtime behavior such as thinking, asking, and
  executing.

## 4.2 Layout

```text
skills/
  role/
  feature/
  system/
```

## 4.3 Prompt Composition

The runtime loads active skills, composes prompt sections, and passes the final
instruction set to the model gateway. Skills should describe behavior and
capabilities, while tool schemas describe callable operations.

Active system, role, and feature Skills form the ordinary tool allowlist.
Registered tools outside that allowlist are not exposed or executable by the
Agent. Background delegation is relation-scoped: after a Skill-authorized
`CreateBackgroundAgentTask` succeeds, Runtime derives the delegator and
assignee from the task and dynamically grants only their lifecycle controls.

Feature Skills may be activated progressively with `GetSkillsList` and
`UpdateSkills`. The update replaces the dynamic feature set, while the stable
role and active system layer remain. Per-Agent retrieval may inject context
automatically before thinking; an active Skill containing `RagRetrieve` adds an
explicit second-pass query without allowing the Agent to switch endpoints.

## 4.4 Best Practices

- Keep role skills focused on identity and policy.
- Keep feature skills focused on capability use.
- Avoid duplicating tool schemas inside skills.
- Use stable terminology: Agent, Runtime, Host, tool, sidecar, snapshot, ledger.
