## Workflow basic syntax

A Workflow is an object containing metadata and `logic`. The `schema` value `orbit.workflow.resource/v1` identifies the format version; it is not a documentation URL. This minimal complete source defines and calls no business nodes:

```text
{
  "schema": "orbit.workflow.resource/v1",
  "id": "example",
  "metadata": {"name": "Echo input", "description": "Return text unchanged.", "extensions": {}},
  "state": "draft",
  "revision": 1,
  "logic": {
    "schema": "orbit.workflow.logic/v1",
    "inputs": {"text": {"type": "String", "description": "Text to return."}},
    "variables": {"done": {"type": "bool", "initial": false}},
    "steps": [{"id": 1, "set": "done", "value": true}],
    "returns": {
      "text": {"value": input.text},
      "done": {"value": var.done}
    }
  }
}
```

1. **Submit** the complete source as a `source` string to the available registration or validation tool. The outer request remains standard JSON. State is draft/registered. New resources start at revision 1; updates submit the current revision and Runtime increments it on commit. Use the promotion operation to promote drafts.
2. **Declare** inputs, variables, and returns as objects keyed by name. Inputs declare type and optional static default. Variables must first declare type and static initial; set targets only a declared variable. Returns contain value. Inputs/returns allow disablename and a short description; variables allow disablename without description. metadata.extensions contains namespaced host objects.
3. **Values and references**: bare expressions are allowed only anywhere inside node args and `returns.<name>.value`. Use `input.name` for input, `numericID.output` for node output, and `var.name` for variables. Registered Pure calls use `registeredFunctionName(arguments, ...)` and may nest. Objects and arrays may recursively mix these values. Quoted strings are always literal text; numbers, booleans, and null are constants. Do not submit internal expression/literal/array/object wrappers there.
4. **Operations**: steps, branches, and loop bodies are ordered arrays. Each item has a globally unique stable positive safe-integer id and exactly one operation below; no op field. Never invent parameter names or outputs.

| Operation | Field shape (replace placeholders) |
| --- | --- |
| Tool/node call | call: registered name string, args: parameter object |
| Condition | if: expression, then: node array, otherwise: node array |
| Loop | for: array expression, body: node array |
| Switch | switch: expression, cases: [{value: constant, body: node array}], default: node array |
| Assignment | set: declared variable name string, value: expression |

5. **Control expressions**: if, for, switch, and set.value still require structured expressions rather than bare expressions. Input: `{"input":"name"}`; output: `{"step":numericID,"output":"outputName"}`; variable: `{"var":"name"}`; Pure: `{"fn":"registeredFunctionName","args":[expression,...]}`. Constant objects use `{"literal":object}`. Names here are syntax placeholders, not node or function definitions.
6. **Scope**: loops have no as field and provide current item/index. Bare args use var.item/var.index; controls use `{"local":"item"}` / `{"local":"index"}`. Index is zero-based. Reference only completed nodes in the current or ancestor scope; branch/loop node outputs cannot escape directly into returns. Switch executes the first matching case, or default, without fallthrough.

Schema checks structure and does not replace these basic syntax rules. Runtime checks order, scope, and contracts. Actual nodes, parameters, outputs, and Pure functions must come from Runtime contracts; inspect tool contracts through ToolDescribe as needed. This syntax defines no node implementations, grants no permissions, and provides no script tools. Invoke only resource operations in current Available Tools.
