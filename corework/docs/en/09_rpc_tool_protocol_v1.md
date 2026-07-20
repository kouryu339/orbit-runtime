# 9 RPC Tool Protocol v1

RPC Tool Protocol v1 is the gRPC protocol used by standalone tool sidecars.

## 9.1 Goals

- Let tools be implemented outside the main Rust runtime.
- Support multiple implementation languages.
- Provide runtime-discoverable tool metadata.
- Return structured outputs that can be shown to the model, the user, or both.

## 9.2 Core Concepts

- **Tool descriptor**: name, description, input schema, and behavior metadata.
- **Tool context**: execution context passed to the tool.
- **AIOutput**: normalized result envelope.
- **HostCall**: request from sidecar to host runtime.
- **HostResult**: host response to a sidecar request.

## 9.3 Service

```text
AgentToolService
  ListTools
  Execute
```

The Rust, Python, Node.js, and C# SDKs expose this protocol through idiomatic
authoring APIs.

## 9.4 Workflow Output Projection

`ToolDescriptor.outputs` is also the workflow node output schema. A successful
`AIOutput.result_json` must be a JSON object containing every registered output
field when that tool is executed as a workflow node. Corework unwraps the
AIOutput envelope and exposes the declared fields directly as pins.

For outputs `page_id` and `url`, return:

```json
{
  "page_id": "page-1",
  "url": "https://example.com"
}
```

Workflow scripts then reference `step.page_id` and `step.url`. `result_json`
does not create a `Result` pin. Missing registered fields are invalid output
and fail execution instead of silently producing null pins.
