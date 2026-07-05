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
