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

## 4.4 Best Practices

- Keep role skills focused on identity and policy.
- Keep feature skills focused on capability use.
- Avoid duplicating tool schemas inside skills.
- Use stable terminology: Agent, Runtime, Host, tool, sidecar, snapshot, ledger.
