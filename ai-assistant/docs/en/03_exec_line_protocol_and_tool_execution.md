# 3 EXEC Line Protocol and Tool Execution

The runtime supports structured tool execution derived from model output.

## 3.1 EXEC Line

An EXEC line is a model-emitted instruction that names a tool and provides
CLI-style named arguments. The parser converts it into a structured runtime
tool call. Parsed parameters go directly to the tool runner without a
serialize-and-reparse round trip, so inline `--script "..."` values preserve
their parsed line breaks and inner quotes. A CLI representation remains only
for audit, approval display, and compatibility recovery.

## 3.2 Execution Flow

```text
model output
  -> parser
  -> tool call
  -> tool runner
  -> built-in system or RPC sidecar
  -> AIOutput
  -> ledger
  -> next model/context step
```

## 3.3 Tool Results

Tool results should use structured fields so the runtime can decide what is
shown to the model, the user, and the host UI.

## 3.4 Terminology

- **Tool call**: a runtime request to execute a named tool.
- **Tool result**: the returned structured output.
- **AIOutput**: the cross-language result envelope.
- **Sidecar**: an out-of-process tool server.
