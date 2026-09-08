# v0.4.8-beta.1 — Workflow execution channels

This Beta refresh adds two-phase workflow execution: call `workflow.start`, subscribe to the returned `workflow_run_id`, then call `workflow.run` with the workflow or temporary script and inputs.

- Stream node start/completion/failure, branch selection, and nested loop iteration facts, correlated by run ID, sequence, and execution IDs.
- Display real tool `to_ai` separately from bounded output-pin previews. End-node outputs arrive in the terminal event.
- Host applications own durable Trace storage and replay. The proposed per-run Runtime journal APIs are not shipped.
- Synchronize Rust, Python, Go, and C++ SDK helpers and public workflow event filtering.
- Expose workflow input metadata and update the English/Chinese execution contracts.
- Fix nested loop name collisions and isolate each run's execution cache so concurrent runs cannot overwrite input pins, variables, or outputs.

Existing synchronous workflow execution remains supported. Node-level approval suspension/resumption is not included. Hosts must establish their event listener before calling `workflow.run`; completion events support results up to 16 MiB.

Validation: workspace tests passed with serial test execution; the concurrent workflow regression passed ten consecutive runs. Native Windows and Linux packages are rebuilt for this Beta refresh. Linux is cross-compiled; local Linux dynamic loading is not part of this validation.
