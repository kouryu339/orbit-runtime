# 4 Blueprint System

The Blueprint system is the node-based workflow layer in `corework`.

## 4.1 Core Model

- **Node**: one executable or evaluatable unit.
- **Exec pin**: carries control flow.
- **Data pin**: carries typed values.
- **Connection**: links compatible pins.
- **Workflow**: a graph of nodes with an entry point.

## 4.2 Node Families

- Control flow: start, end, branch, loop, break.
- Data: math, logic, string, array, object, variable, conversion.
- System: dynamic calls into registered systems.
- Utility and placeholder nodes for debugging and migration.

## 4.3 Type Safety

The pin system supports concrete types and wildcard types. Wildcards allow
generic nodes, while type inference resolves them when connections are made.

## 4.4 Runtime

The execution engine tracks node state, stack frames, current flow position, and
shared scoped data.

## 4.5 Dynamic Tool Output Pins

Dynamic RPC system nodes derive data pins from `RuntimeToolMetadata.outputs`.
The RPC `AIOutput` envelope is not a node schema: Corework unwraps
`AIOutput.result`, validates the registered fields, and writes each field to its
declared output pin. A tool declaring `page_id` and `url` therefore exposes
those two pins, not a synthetic `Result` pin.

Chain scripts reference the declared fields directly:

```text
1: BrowserOpenPage --url $url
return page_id=1.page_id url=1.url
```

The final `return` compiles to End-node outputs. Only those End outputs become
the workflow program result; intermediate node outputs remain internal unless
explicitly returned.
