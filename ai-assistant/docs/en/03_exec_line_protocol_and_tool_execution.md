# 3 EXEC Line Protocol and Tool Execution

The runtime supports structured tool execution derived from model output.

## 3.1 EXEC Line

An EXEC line is a model-emitted instruction that names a tool and provides JSON
arguments. The parser converts that instruction into a runtime tool call.

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
