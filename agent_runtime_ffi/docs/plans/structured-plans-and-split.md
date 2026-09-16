# Structured execution plans and literal splitting

## Plan tools

Plans belong to the calling Agent's cache. Agent cache checkpoints and
`conversation.state_delta` (`agent_plan.set`) carry the same complete state,
including `created_at`, `plan_id`, `revision` and `steps`. The tools no longer
write another copy to the process-wide default Agent's SessionMeta.
Hosts that persist conversation state must store these deltas alongside the
ledger and provide them on snapshot import.

Create:

```json
{"title":"Ship","summary":"Verify and publish","steps":[
  {"id":"verify","text":"Verify the change","status":"in_progress"},
  {"id":"publish","text":"Publish the build","status":"pending"}
]}
```

Update one step with `PlanUpdate`:

```json
{"plan_id":"<returned id>","revision":1,"step_id":"verify","step_status":"completed"}
```

Pass the latest returned revision on subsequent calls. Replace `steps` to
reorder/add/remove steps, preserving existing IDs. At most one step may be
`in_progress`. Other states are `pending`, `completed`, `blocked`, `canceled`.
Finish with `PlanFinish(plan_id, revision)` only after every step is completed;
explicit abandonment uses `status: "canceled"`. An active plan cannot be
overwritten by PlanWrite.

Old Markdown-only saved plans still load. Old event payloads missing created_at
use updated_at on import. New updates/finish require plan_id and revision;
legacy plans without IDs retain compatibility. Active structured plans inject
their ID, revision, title, summary and steps at the end of each model request.
Full Markdown detail remains in state rather than repeating in the prompt.

Lit displays the focused Agent's plan in one collapsible card. Null snapshots
clear it; background-agent deltas do not replace it. Closed plans remain visible.

## Split Pure node

`SplitNode`: `Value: String`, `Separators: Array<String>` →
`Parts: Array<String>`.

```text
input text:String
return parts=split(input.text, ["-", ",", "，"])
```

`aa-c,i` returns `["aa", "c", "i"]`.
Separators are literal strings (including multi-character delimiters), not
regular expressions. At each position the longest matching delimiter wins.
Empty fields and whitespace are preserved: `a--b` split on `["-"]` gives
`["a", "", "b"]`. No separators returns the original string as one element;
an empty input returns `[""]`. Empty delimiter strings and non-string
delimiter elements are rejected.
