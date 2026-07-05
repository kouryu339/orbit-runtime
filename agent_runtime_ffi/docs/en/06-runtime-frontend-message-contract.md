# 6 Runtime Frontend Message Contract 0.3.0

This document defines the stable frontend-renderable message contract inside `frontend:state_snapshot`. It describes runtime-to-frontend display messages, not tool execution internals, LLM telemetry, or business-side SSE events.

## 6.1 Version Status

`0.3.0` is a major update to the frontend message contract. The previous frontend message contract is deprecated; new frontend, test-platform, and host integrations should follow this document.

This deprecation only applies to the frontend-renderable message shape. It does not deprecate the `agent-runtime-event/v1` envelope schema, FFI function suffixes such as `_v1`, or other config schemas. Those names remain governed by their own docs and code compatibility.

## 6.2 Canonical Channel

The frontend should treat only `agent-runtime-event/v1` events with
`type == "frontend:state_snapshot"` as the source of frontend-renderable
conversation messages. Runtime status deltas such as task, plan, focus, and
dynamic snapshot updates may be consumed from `conversation.state_delta`.
`conversation.ledger_delta` belongs to host persistence/recovery and must not
enter the frontend rendering protocol.

```text
payload.ledger_delta.record
```

is the optional message delta carried by the snapshot. When `ledger_delta` is absent, the event only means state, capability flags, or other snapshot fields changed.

## 6.3 Message Roles

| `record.role` | Frontend use |
|---|---|
| `user` | User message echo. |
| `assistant` | Main chat bubble. Render `record.content`. |
| `gateway_message` | Tool start, routing hint, or system progress. Usually render in a tool/progress panel. |
| `tool` | Tool completion result. Usually render in a tool/progress panel or collapsed result area. |
| `agent_report` | Sub-agent or task report. Render according to product needs. |
| `summary` | Context compaction summary. Usually not shown to the user. |

The frontend should not use the outer SSE `type` to decide whether a message is an assistant message or a tool message. Message meaning comes from `record.role` and `record.metadata.subtype`.

## 6.4 Assistant Display Content

For `assistant` records, `record.content` is the frontend display text. Runtime applies a display projection before sending `ledger_delta`:

- `<think>...</think>` does not enter frontend display content.
- Raw `EXEC ...` tool-call lines are not shown directly.
- Each visible tool-call position is projected into a tool status placeholder.
- Widget tags may remain as standalone lines for the frontend to render as input controls.
- LLM natural-language content may use Markdown; the frontend should render it with the unified content-block rules in this document.

Tool status placeholder syntax:

```text
[tool:status | call_id="<agent_id>:<turn_id>:<index>"]
```

The frontend should render this as the status of the corresponding tool call, not as ordinary text. Status data comes from subsequent or already received `gateway_message` / `tool` records correlated by the same `call_id`.

## 6.5 Unified Content Block Rendering

`record.content` may contain three kinds of frontend-visible content at the same time:

| Content type | Source | Frontend handling |
|---|---|---|
| Tool status placeholder | Runtime projects `EXEC` into `[tool:status | call_id="..."]` | Render as a tool status component and correlate start/end records by `call_id`. |
| Widget tag | Standalone control line emitted by assistant/runtime | Render as an input control; submit values back as an ordinary user message. |
| Markdown body | Natural LLM output such as explanation, results, code, or tables | Render as standard content components, not plain text. |

Recommended parsing order for one `assistant` message:

1. Scan for standalone protocol lines.
2. When `[tool:status | call_id="..."]` is found, extract it and render a tool status component.
3. When a Widget tag is found, extract it and render a control component.
4. Parse the remaining body as Markdown and render standard content blocks.
5. Preserve the original order when composing Markdown blocks, tool status components, and Widget controls into one assistant message.

Markdown is an LLM text-formatting capability, not a runtime control protocol. The frontend must treat Markdown as display-layer formatting and must not infer business actions or tool calls from Markdown text.

### 6.5.1 Markdown Support Scope

The 0.3.0 frontend should support at least these Markdown blocks:

| Markdown shape | Frontend component |
|---|---|
| `#` / `##` / `###` headings | Heading block; preserve hierarchy without breaking chat bubble hierarchy. |
| Plain paragraphs | Paragraph text with readable line height. |
| `-` / `*` unordered lists | List component. |
| `1.` ordered lists | Ordered list component. |
| `>` quotes | Quote block. |
| `` `inline code` `` | Inline code style. |
| Triple-backtick code fences | Code block component, horizontally scrollable, optionally showing language label. |
| Markdown tables | Table component, horizontally scrollable. |
| `---` separators | Divider. |
| `[label](https://example.com)` links | Link component; allow only safe protocols such as `http`, `https`, and `mailto`. |
| Fenced `mermaid` block | Mermaid diagram; use strict security mode and fall back to a code block on failure. |
| `$...$`, `$$...$$`, `\(...\)`, `\[...\]` | Inline or display LaTeX formula; render with trust disabled and fall back to text on failure. |

The frontend should not inject unsanitized Markdown-generated HTML into the page. Prefer parsing Markdown into structured blocks and rendering them through native framework components to avoid script injection, style escape, and layout pollution.

### 6.5.2 Boundary Between Protocol Lines And Markdown

`[tool:status | call_id="..."]` and Widget tags are protocol lines only when they occupy a full standalone line. The same text inside a paragraph, code block, table cell, or inline code must be displayed as ordinary Markdown text.

Example:

```text
I will analyze the file first, then show a table.
[tool:status | call_id="runtime-agent:12:0"]

## Recommended Results

| Format | Use case |
|---|---|
| MP3 | Car audio and common playback |
| FLAC | Lossless archive |

Choose an export format:
[select:single | label="Format" | options="MP3,FLAC"]
```

The frontend should render:

1. Markdown paragraph: "I will analyze the file first, then show a table."
2. Tool status component.
3. Markdown heading and table.
4. Widget single-select control.

## 6.6 Tool Call Records

The tool lifecycle is represented by separate ledger records:

| `metadata.subtype` | Common `role` | Meaning |
|---|---|---|
| `tool_call_permission_requested` | `gateway_message` | Tool execution is paused before running and is waiting for user approval. |
| `tool_call_started` | `gateway_message` | Tool execution started. |
| `tool_call_finished` | `tool` | Tool execution completed successfully. |
| `tool_call_failed` | `tool` | Tool execution failed. |

`metadata.extra.call_id` in these records must match the `[tool:status | call_id="..."]` placeholder in assistant content. The frontend uses `call_id` to merge the placeholder, permission request, start event, and finish event for the same tool call.

Do not infer tool state from message arrival order alone. Consume snapshots by `revision` / `conversation_event_seq`, then update the matching placeholder by `call_id`.

### 6.6.1 Tool Control State

Tool controls show what the agent is doing. They have three sources:

| Source | Meaning |
|---|---|
| `[tool:status | call_id="..."]` inside `assistant.content` | Display anchor. When only the anchor is known, the frontend may render a placeholder state. |
| `gateway_message` with `metadata.subtype = tool_call_permission_requested` | The tool has not run yet and is waiting for a user decision. If `frontend:state_snapshot.payload.pending_permissions` contains the same `tool_call_id`, render the allow/deny controls inside this tool control. |
| `gateway_message` with `metadata.subtype = tool_call_started` | The tool started running. |
| `tool` with `metadata.subtype = tool_call_finished` / `tool_call_failed` | The tool ended, successfully or with failure. |

The frontend merges these into one tool control by `call_id`. Recommended minimal UI state:

```ts
type ToolUiState =
  | "placeholder"         // only the assistant placeholder has been seen
  | "waiting_permission"  // tool_call_permission_requested has been received
  | "running"             // tool_call_started has been received
  | "finished"            // tool_call_finished has been received
  | "failed";             // tool_call_failed has been received
```

Suggested rendering:

```text
placeholder:
○ Preparing tool call

running:
⟳ Running ProductSearch

finished:
✓ ProductSearch completed
  Found 5 results.

failed:
! ProductSearch failed
  Missing query parameter.
```

Tool result details such as `to_ai` or `record.content` belong inside the tool control details area and should not be rendered as a normal assistant bubble.

## 6.7 Mixed Tools And Widgets

The `0.3.0` contract allows one assistant display message to contain natural language, tool status placeholders, and Widget tags, for example:

```text
I will read the document first and prepare the choices.
[tool:status | call_id="runtime-agent:12:0"]
Choose an export format:
[select:single | label="Format" | options="PDF,DOCX"]
```

Semantics:

1. The assistant first shows explanatory text and tool status.
2. Runtime executes the corresponding tool call.
3. If the same display content contains a Widget, runtime waits for user input after tool execution completes.
4. The frontend collects Widget values and sends them back to runtime as an ordinary user message.

Widget tags are still not tool calls. They must not be written inside an `EXEC` line or used as tool arguments. They are only a frontend rendering protocol.

## 6.8 Widget Tags

Widget tags are part of the `0.3.0` frontend message contract. They may appear in two frontend-visible text surfaces:

- the `prompt` field of an `asking` decision;
- standalone lines inside `assistant` display content, where they may coexist with `[tool:status | call_id="..."]` placeholders.

Widget tags are not tool calls and must not be written as `EXEC`. After submission, the frontend converts entered values into a normal user message and sends that message back to runtime.

### 6.8.1 Basic Syntax

Each Widget tag must occupy its own line:

```text
[kind:type | key="value" | key2="value2"]
```

Syntax rules:

- A tag starts with `[` and ends with `]`.
- The first segment is the control type, such as `input:text`, `select:single`, or `confirm`.
- Additional fields are separated by vertical bars `|`.
- Field syntax is `key="value"`.
- Field values must use double quotes.
- `label` is required.
- Widget tags may be mixed with natural-language text, but every Widget tag must be on its own line.
- Non-tag lines are rendered as ordinary display text.

### 6.8.2 Widget Types

| Tag | Meaning | Required fields | Optional fields | User submission format |
|---|---|---|---|---|
| `[input:text | label="Name"]` | Text input. | `label` | None | `Name: user input` |
| `[input:path | label="Video" | accept=".mp4,.mov"]` | File or path picker. | `label` | `accept` | `Video: D:/xxx.mp4` |
| `[input:date | label="Date"]` | Date picker. Value format is `YYYY-MM-DD`. | `label` | None | `Date: 2025-01-15` |
| `[input:time | label="Time"]` | Time picker. Value format is `HH:MM`. | `label` | None | `Time: 14:30` |
| `[select:single | label="Format" | options="MP4,AVI,MKV"]` | Single select. | `label`, `options` | None | `Format: MP4` |
| `[select:multi | label="Tags" | options="Funny,Life,Tech"]` | Multi select. | `label`, `options` | None | `Tags: Funny, Life` |
| `[confirm | label="Operation details"]` | Confirm or cancel. | `label` | None | `Operation details: yes` or `Operation details: no` |

### 6.8.3 Fields

| Field | Meaning |
|---|---|
| `label` | The user-facing control label. It is also used as the field name in the submitted user message. Keep it short and clear. |
| `accept` | Only for `input:path`. A comma-separated list of suggested file extensions, such as `.mp4,.mov,.avi`. |
| `options` | Only for `select:single` and `select:multi`. Options must be separated with English commas, such as `MP4,AVI,MKV`. |

### 6.8.4 Submission Semantics

After the user submits controls, the frontend does not send the original Widget tags back. It converts each value into ordinary user text:

```text
Format: MP4
Date: 2025-01-15
```

Multi-select values are displayed with the product's chosen separator, for example:

```text
Tags: Funny, Life
```

The assistant should read the submitted content as a normal user message.

### 6.8.5 Widget Control State

Widget controls collect what the user should provide next. They only come from standalone Widget lines inside assistant content. They are not tool calls, do not use `call_id`, and do not wait for tool lifecycle events.

Recommended minimal UI state:

```ts
type WidgetUiState =
  | "enabled" // Widget in the latest assistant message; user has not acted yet
  | "ready"   // user has entered, selected, or clicked enough to submit
  | "expired"; // a newer assistant message arrived, so old Widgets are no longer active
```

Lifecycle:

```text
assistant A arrives with Widget
-> enabled

user enters/selects/clicks
-> ready

user submits
-> submit only the ready Widget values from this turn, convert them to a normal user message, and send it back to runtime

assistant B arrives
-> all Widgets in assistant A become expired
```

No matter how many Widgets appear in the same assistant turn, the frontend should render only one submit button for the WidgetPanel. When the user clicks submit:

- ready Widget values are converted into a normal user message;
- non-ready Widgets are not included in the submitted message;
- the current WidgetPanel enters the submitted state, and non-ready Widgets become expired, read-only, or disabled;
- when a newer assistant message arrives, all unsubmitted Widgets from the previous assistant turn also become expired.

Submission example:

```text
Format: PDF
Date: 2026-05-28
Confirm delete D:/tmp/a.txt: yes
```

Do not send the original Widget tags back to runtime.

### 6.8.6 Writing Requirements

- Generate controls dynamically from context. Do not use fixed filler text.
- When no structured input is needed, ask with plain natural language and do not force Widget tags.
- For risky operations, use `confirm` and include the file name, action, and key parameters in the label or surrounding text.
- `select` `options` must be separated by English commas. Do not concatenate all options into one string without separators.
- Widget tags may coexist with tool status placeholders, but must not be placed inside `EXEC` lines or used as tool arguments.

## 6.9 Frontend Rules

- Render `record.role == "user"` as a right-aligned user bubble.
- Render `record.role == "assistant"` as a left-aligned content stream rather than a heavy enclosing bubble.
- When `[tool:status | call_id="..."]` appears, create or update a tool status component.
- When a Widget tag appears, render it according to the Widget tag protocol in this document.
- Do not render tool start/finish records as ordinary assistant bubbles.
- Do not place `gateway_message` or `tool` records in the main chat stream. Use them to update tool status components, or show them in a separate debug/progress panel.
- Do not render summary records as normal chat content by default.
- Use `payload.conversation_state` as the canonical cluster-level runtime phase.
- Enable the composer and manual compaction only when `conversation_state === "waiting"`.
- Enable pause when the state is `thinking` or `executing`.
- A turn is complete only after that turn has entered a non-`waiting` state and later returned to `waiting`, with all observed tool calls settled.

## 6.10 Recommended Chat Rendering

Frontend implementations should treat user and assistant records as different visual semantics:

- User messages are explicit input units. Render them as compact right-aligned bubbles, with a recommended maximum width of about `70%`.
- Assistant messages are readable and actionable content canvases. Render them as left-aligned content streams, with a recommended maximum width of about `760px`.

Recommended component structure:

```text
ConversationView
  MessageList
    UserBubble
    AssistantMessage / ContentStream
      MarkdownBlock
      ToolStatusInline
      WidgetControl
      CodeBlock
      TableBlock
  Composer disabled={snapshot.conversation_state !== "waiting"}
```

Visual rules:

- User messages should be right-aligned and use a clear bubble background, such as a brand color or dark fill. They should only contain user text or submitted Widget values.
- Assistant messages should be left-aligned, but should not be wrapped in a heavy enclosing bubble. The container may provide an avatar, spacing, a left rail, and max width.
- Assistant paragraphs, headings, lists, tables, and code blocks should be rendered as independent Markdown content blocks.
- Tool status, Widget controls, code blocks, and tables may use lightweight containers. These are content block styles, not separate chat bubbles.
- Tool status should not appear in a modal or overlay the assistant text. Render it as an inline timeline or status row at the position of `[tool:status | call_id="..."]`. After completion, it may collapse to one summary row, such as `Completed: ReadDocument 1.2s`.
- Widget controls are part of the assistant content stream and should look like embedded forms. After submission, the frontend should convert values into ordinary user text and render that text as a right-aligned user bubble.

One-line principle: the user is a bubble, the assistant is a content canvas; tools are status rows, Widgets are embedded controls; everything is placed by `record.role` and `call_id`.
