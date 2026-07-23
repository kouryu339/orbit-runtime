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
input url:String
1: EXEC BrowserOpenPage --url input.url
return page_id=1.page_id url=1.url
```

The final `return` compiles to End-node outputs. Only those End outputs become
the workflow program result; intermediate node outputs remain internal unless
explicitly returned.

## 4.6 Chain Script Boundaries And Step Numbers

Canonical layout:

```text
input a:num=1 b:String="a" c:bool
$variable = literal
1: EXEC ToolName --count input.a --label input.b --checked input.c
return output=1.output_pin
```

- Prefer one `input` declaration containing every field so the complete input
  contract is visible at once. The compiler accepts consecutive `input` lines
  for compatibility and merges them into one Start node, but new scripts
  should not generate that form. Input field names must be unique.
- One or more consecutive `return` declarations must appear at the end. The
  compiler merges all outputs into the End node and rejects duplicates.
- Input types are limited to `num`, `String`, `bool`, `Any`, and recursive
  `Array<T>` forms such as `Array<String>`, `Array<Array<num>>`, and
  `Array<Any>`. `Any` accepts any existing expression result but does not add
  a new object-literal syntax.
- External tools use `N: EXEC ToolName --param value`. Pure expressions are
  embedded in arguments, conditions, `setvar`, or `return` and have no number.
- Top-level executable steps are unique consecutive integers: `1`, `2`, `3`.
- Nested numbering appears only inside a branch or loop body. An IF body step
  is `current IF number.branch code.step ordinal`; a FOR body step is
  `current FOR number.body step ordinal`. Ordinals start at `1`.
- IF branch codes are `1` for the first true branch, `2`, `3`, ... for ELIF,
  and `0` for ELSE. `END` has no number.
- Numbers cannot repeat, skip an expected ordinal, or contain letters. Step
  references use the same identifier, such as `2.page_id`.
- `input.name`, `$variable`, and `N.pin` are dynamic references and must not be
  quoted. Quoted values are always fixed strings.
