# Workflow resource public API

Six resource operations are exposed. Existing create/read/revise/update/compile/test/script and two-phase run commands are deprecated and disabled.

| AI tool | Runtime command | Parameters |
|---|---|---|
| listWorkflows | workflow.list | optional kind: all (default), draft, registered |
| registerWorkflow | workflow.register | source: complete resource source text |
| removeWorkflow | workflow.remove | workflow_id / id, expected_revision |
| validateWorkflow | workflow.validate | source: complete resource source text |
| promoteWorkflow | workflow.promote | workflow_id / id, expected_revision |
| executeWorkflow | workflow.execute | workflow_id / id, inputs, optional trace |

## Public schemas and source syntax

Distribute these files alongside the DLL from the Runtime package:

| Package path | Purpose |
|---|---|
| `schemas/resource.schema.json` | Complete resource structure, including the embedded logic schema |
| `schemas/logic.schema.json` | Standalone logic structure |
| `docs/en/workflow-json-source-syntax.md` | English source syntax and minimal example |
| `docs/zh/workflow-json-source-syntax.md` | Chinese source syntax and minimal example |

Repository sources are `corework/src/workflow/{resource,logic}.schema.json` and `ai-assistant/prompts/{en,zh}/workflow_tool_rules.md`. The schemas use JSON Schema Draft 2020-12 with IDs `urn:orbit:workflow:resource:v1` and `urn:orbit:workflow:logic:v1`. These IDs are identifiers, not download URLs. The resource schema embeds the logic definition and does not require a network fetch.

Schemas validate the parsed, normalized structure, not raw source containing bare expressions such as `input.name`. Hosts should submit authored source to `workflow.validate`, which also checks contracts, references and scopes. There is currently no Runtime command or AI tool for retrieving schemas; the files are public package artifacts. The brief syntax guide embedded in thinking-pro lets the model learn the basics directly.

## Resource documents

Use orbit.workflow.resource/v1 documents containing id, metadata (name, description, namespaced extensions), state (draft or registered), revision, and logic. Node args and return values support the existing bare-expression source syntax. The outer command remains valid JSON because source is a string.

A new identity starts at revision 1. Registration replaces the full existing document only when its revision matches the accepted version; Runtime increments the committed revision. Initial state comes from the document. Editing an existing identity saves it as draft, including edits submitted with its previous registered header. Draft-to-registered transitions require promote. Namespaced metadata survives promotion and persistence.

Active executions block edits, removal and promotion with workflow_in_use. Occupancy covers the root and recursive references until completion or cancellation. Drafting a previously registered resource withdraws its reference definition. Formal callers remain visible with production_executable:false and invalid validation; execution recursively rejects unstable dependencies. Promotion restores availability. Startup preserves blocked callers. Detect capabilities.features.workflow_edit_to_draft and workflow_running_edit_guard, both true.

Validation uses the same compilation and reference checks without writing or executing. Execution selects draft or production from accepted state; no mode or separate draft-test tool is needed. AI execution permissions and paired conversation_id/agent_id context still apply. Removal and promotion require expected_revision. Removal deletes the corresponding source file inside the Runtime-managed directory, including manually imported sources; keep author backups elsewhere. Breaking reference contracts are rejected.

Runtime persists complete source text as `<id>.workflow.json`. Startup restores `.json` and `.workflow` resource documents after installing tool contracts, compiles them in dependency order, and preserves state and revision. Startup does not write files or execute nodes. Legacy scripts and graphs are skipped with a log message and remain unavailable; no automatic conversion is performed. Invalid new documents, duplicate identities, and invalid references fail startup without overwriting the source files.

See the [complete contract and example](../zh/12-workflow-resource-api.md).

## Deprecated standalone pages

Workflow Studio and Agent Test Studio are deprecated. studio.open_workflow and studio.open_agent_test are absent from capabilities and return deprecated_api when invoked. Internal page implementations remain for compatibility; their migration is outside this delivery.

`workflow.list` / `listWorkflows` remains available as a read-only resource catalog. Omitted, null, or all kind lists both states; draft and registered filter by state. Invalid filters are rejected. Results contain summary identity, metadata, kind, revision and validation, without source or execution.
