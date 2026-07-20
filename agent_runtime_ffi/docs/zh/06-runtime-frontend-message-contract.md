# 6 Runtime 前端消息契约 0.3.0

本文定义 `frontend:state_snapshot` 中前端可渲染消息的稳定契约。它描述的是
runtime 到前端的展示层消息形态，不替代工具执行、LLM telemetry 或业务侧 SSE
协议。

## 6.1 版本状态

`0.3.0` 是前端消息契约的大版本更新。旧版前端消息契约已经废弃，前端、测试平台
和宿主侧新接入应以本文档为准。

这里的“旧版契约废弃”只指前端可渲染消息形态，不表示
`agent-runtime-event/v1` envelope schema、FFI 函数名中的 `_v1` 后缀或其他配置
schema 同步废弃。那些名称仍按各自文档和代码兼容性执行。

## 6.2 消费通道

前端只应把 `agent-runtime-event/v1` 且 `type == "frontend:state_snapshot"` 的事件
作为对话可渲染消息来源；任务、计划、focus、动态快照等运行状态增量可消费
`conversation.state_delta`。`conversation.ledger_delta` 属于宿主持久化和恢复通道，
不应直接成为聊天渲染协议。

```text
payload.ledger_delta.record
```

是本次快照携带的可选消息增量。没有 `ledger_delta` 时，事件只表示状态、能力位或
其他快照字段发生变化。

## 6.3 消息角色

| `record.role` | 前端用途 |
|---|---|
| `user` | 用户消息气泡。前端渲染 `record.content`。 |
| `assistant` | 主聊天内容流。前端渲染 `record.content`。 |
| `gateway_message` | 工具开始、路由提示、系统进度。通常进入工具或进度面板。 |
| `tool` | 工具结束结果。通常进入工具详情或折叠结果区。 |
| `agent_report` | 子 Agent 或任务报告，按产品形态渲染。 |
| `summary` | 上下文压缩摘要，默认不展示给用户。 |

前端不要用 SSE 外层 `type` 判断“这是助手消息还是工具消息”。消息含义以
`record.role` 和 `record.metadata.subtype` 为准。

## 6.4 Assistant 内容

`assistant` 记录的 `record.content` 是前端展示文本。Runtime 会在下发
`ledger_delta` 前应用展示投影：

- `<think>...</think>` 不进入前端展示内容；
- 原始 `EXEC ...` 工具调用行不直接展示；
- 每个可见工具调用位置会投影成工具状态占位符；
- Widget 控件标签可以保留为独占行，供前端渲染为输入控件；
- LLM 自然语言正文可以使用 Markdown，前端应按本文档的统一内容块规则渲染。

工具状态占位符语法：

```text
[tool:status | call_id="runtime-agent:12:0"]
```

前端应把它渲染为对应工具调用的运行状态，而不是普通文本。状态数据来自后续或已有的
`gateway_message` / `tool` 记录，并通过同一个 `call_id` 关联。

`record.content` 中可能同时包含三类前端可见内容：

| 内容类型 | 来源 | 前端处理 |
|---|---|---|
| 工具状态占位符 | Runtime 将 `EXEC` 投影为 `[tool:status | call_id="..."]` | 渲染为工具状态组件，并用 `call_id` 关联工具开始和结束记录。 |
| Widget 控件标签 | Assistant/runtime 输出的独占控件行 | 渲染为输入控件；提交后转换为普通用户消息。 |
| Markdown 正文 | LLM 自然输出的说明、结果、代码、表格等 | 渲染为标准内容组件，而不是纯文本。 |

推荐解析顺序：

1. 按行扫描独占协议行。
2. 命中 `[tool:status | call_id="..."]` 时，抽出并渲染为工具状态组件。
3. 命中 Widget 控件标签时，抽出并渲染为控件组件。
4. 剩余正文按 Markdown 解析为标准内容块。
5. 保持原有顺序，把 Markdown 内容块、工具状态组件和 Widget 控件组合成同一条
   assistant 消息。

Markdown 是 LLM 的文本表达能力，不是 runtime 控制协议。前端必须把 Markdown 当作
展示层格式处理，不得从 Markdown 文本反推出业务动作或工具调用。

## 6.5 Markdown 内容块

0.3.0 前端至少应支持以下 Markdown 内容块：

| Markdown 形态 | 前端组件 |
|---|---|
| `#` / `##` / `###` 标题 | 标题块，保持层级但不要破坏聊天视觉层级。 |
| 普通段落 | 段落文本，保留换行语义和可读行高。 |
| `-` / `*` 无序列表 | 列表组件。 |
| `1.` 有序列表 | 列表组件。 |
| `>` 引用 | 引用块。 |
| `` `inline code` `` | 行内代码样式。 |
| 三反引号代码块 | 代码块组件，横向可滚动，可显示语言标签。 |
| Markdown 表格 | 表格组件，横向可滚动。 |
| `---` 分隔线 | 分隔线。 |
| `![alt](https://example.com/image.png)` 图片 | 图片组件；只允许安全协议和受信任来源。 |
| `[label](https://example.com)` 链接 | 链接组件；只允许 `http`、`https`、`mailto` 等安全协议。 |
| `mermaid` 围栏代码块 | Mermaid 图表；使用严格安全模式，失败时回退为代码块。 |
| `$...$`、`$$...$$`、`\(...\)`、`\[...\]` | 行内或块级 LaTeX 公式；关闭 trust，失败时回退为文本。 |

前端不应直接把 Markdown 转成未净化的 HTML 插入页面。推荐解析为结构化 block，再由
框架原生组件渲染，避免脚本注入、样式逃逸和布局污染。

`[tool:status | call_id="..."]` 和 Widget 控件标签只有在独占一行时才是协议行。
出现在普通段落、代码块、表格单元格或行内代码中的相同文本，应按 Markdown 普通文本
展示。

示例：

```markdown
我先分析文件，然后给你一个表格。

[tool:status | call_id="runtime-agent:12:0"]

## 结果

| 格式 | 说明 |
|---|---|
| MP3 | 车载、通用播放 |
| FLAC | 无损归档 |

请选择导出格式：
[select:single | label="格式" | options="MP3,FLAC"]
```

前端应渲染为：

1. Markdown 段落“我先分析文件，然后给你一个表格。”
2. 工具状态组件。
3. Markdown 标题和表格。
4. Widget 单选控件。

## 6.6 工具生命周期记录

| `metadata.subtype` | 常见 `role` | 含义 |
|---|---|---|
| `tool_call_permission_requested` | `gateway_message` | 工具执行前暂停，等待用户确认。 |
| `tool_call_started` | `gateway_message` | 工具开始执行。 |
| `tool_call_finished` | `tool` 或 `gateway_message` | 工具成功结束。 |
| `tool_call_failed` | `tool` 或 `gateway_message` | 工具失败。 |
| `tool_call_permission_resolved` | `gateway_message` | 用户已允许或拒绝。 |

这些记录的 `metadata.extra.call_id` 必须与 assistant 内容里的
`[tool:status | call_id="..."]` 对齐。前端使用 `call_id` 合并同一次工具调用的
占位符、权限申请、开始事件和结束事件。

前端不要依赖消息到达顺序推断工具状态；应按 `revision` / `conversation_event_seq`
接收快照，并用 `call_id` 更新对应占位符。

### 6.6.1 工具控件状态

工具控件展示 Agent 正在做什么。它有三类来源：

| 来源 | 前端含义 |
|---|---|
| `assistant.content` 中的 `[tool:status | call_id="..."]` | 展示锚点。只看到锚点时，前端可先渲染占位状态。 |
| `gateway_message` + `metadata.subtype = tool_call_permission_requested` | 工具尚未执行，正在等待用户确认。对应工具控件保持 `waiting_permission`；允许/拒绝操作根据 canonical `pending_permissions` 统一渲染在输入框上方的 approval shelf。 |
| `gateway_message` + `metadata.subtype = tool_call_started` | 工具开始执行。 |
| `tool` / `gateway_message` + finished/failed subtype | 工具结束，控件进入完成或失败状态。 |

前端用同一个 `call_id` 合并为一个工具控件。推荐最小 UI 状态：

```ts
type ToolCallUiState =
  | "placeholder"        // only the assistant placeholder has been seen
  | "waiting_permission" // permission request is pending
  | "running"            // started but not settled
  | "finished"           // completed successfully
  | "failed";            // completed with error
```

推荐文案：

```text
placeholder:
  Preparing ProductSearch

waiting_permission:
  ProductSearch needs approval
  [Allow] [Deny]

running:
  Searching products...

finished:
  Completed ProductSearch
  Found 5 results.

failed:
  ProductSearch failed
  Network timeout.
```

工具结果里的 `to_ai` 或 `record.content` 只放进工具控件详情里，不再渲染成普通
assistant 气泡。

## 6.7 混合工具与控件

`0.3.0` 契约允许一条 assistant 展示消息同时包含自然语言、工具状态占位符和 Widget
控件标签，例如：

```markdown
我会先检查文件，然后让你确认导出格式。

[tool:status | call_id="runtime-agent:23:0"]

请选择导出格式：
[select:single | label="格式" | options="PDF,DOCX"]
```

处理顺序：

1. Assistant 先展示说明文本和工具状态。
2. Runtime 执行对应工具调用。
3. 如果同一展示内容中包含 Widget，工具执行结束后 runtime 进入等待用户输入状态。
4. 前端按 Widget 协议收集值，并把结果作为普通用户消息发回 runtime。

Widget 标签不是工具调用，不能写在 `EXEC` 行里，也不能作为工具参数的一部分。它们只是
前端渲染协议。

## 6.8 Widget 控件标签

Widget 控件标签是 `0.3.0` 前端消息契约的一部分。它可以出现在两类前端可见文本中：

- `asking` 决策的 `prompt` 字段；
- `assistant` 展示内容中的独占行，可与 `[tool:status | call_id="..."]` 工具状态
  占位符共存。

控件标签不是工具调用，不得写成 `EXEC`。控件提交后，前端把用户填写的值转换为普通
用户消息再发回 runtime。

### 6.8.1 语法

控件标签必须独占一行：

```text
[input:text | label="姓名"]
[input:path | label="视频" | accept=".mp4,.mov"]
[input:date | label="日期"]
[input:time | label="时间"]
[select:single | label="格式" | options="MP4,AVI,MKV"]
[select:multi | label="标签" | options="搞笑,生活,科技"]
[confirm | label="确认删除 D:/tmp/a.txt"]
```

- 标签以 `[` 开始，以 `]` 结束。
- 第一段是控件类型，例如 `input:text`、`select:single` 或 `confirm`。
- 后续字段用竖线 `|` 分隔。
- 字段格式为 `key="value"`。
- 字段值必须使用双引号。
- `label` 是必填字段。
- 控件标签可以和自然语言混排，但每个控件标签必须单独占一行。
- 非控件行按普通展示文本渲染。

### 6.8.2 控件类型

| 标签 | 含义 | 必填字段 | 可选字段 | 用户提交格式 |
|---|---|---|---|---|
| `[input:text | label="姓名"]` | 文本输入框。 | `label` | 无 | `姓名: 用户输入` |
| `[input:path | label="视频" | accept=".mp4,.mov"]` | 文件或路径选择。 | `label` | `accept` | `视频: D:/xxx.mp4` |
| `[input:date | label="日期"]` | 日期选择。值格式为 `YYYY-MM-DD`。 | `label` | 无 | `日期: 2025-01-15` |
| `[input:time | label="时间"]` | 时间选择。值格式为 `HH:MM`。 | `label` | 无 | `时间: 14:30` |
| `[select:single | label="格式" | options="MP4,AVI,MKV"]` | 单选。 | `label`, `options` | 无 | `格式: MP4` |
| `[select:multi | label="标签" | options="搞笑,生活,科技"]` | 多选。 | `label`, `options` | 无 | `标签: 搞笑, 生活` |
| `[confirm | label="操作说明"]` | 确认或取消。 | `label` | 无 | `操作说明: yes` 或 `操作说明: no` |

### 6.8.3 字段

| 字段 | 含义 |
|---|---|
| `label` | 展示给用户的控件标题，也作为提交回填消息中的字段名。必须简短、明确。 |
| `accept` | 仅用于 `input:path`。表示建议选择的文件扩展名列表，使用英文逗号分隔，例如 `.mp4,.mov,.avi`。 |
| `options` | 仅用于 `select:single` 和 `select:multi`。选项必须使用英文逗号 `,` 分隔，例如 `MP4,AVI,MKV`。 |

### 6.8.4 提交语义

用户提交控件后，前端不会把原始 Widget 标签发回。前端会把每个控件值转换为普通用户文本：

```text
格式: MP4
日期: 2025-01-15
```

多选值使用产品约定的分隔符展示，例如：

```text
标签: 搞笑, 生活
```

Assistant 后续应把这些内容当作普通用户输入读取。

### 6.8.5 Widget 控件状态

Widget 控件收集用户下一步要提供什么。它只来自 assistant 内容里的独占 Widget 行，不属于
tool call，不使用 `call_id`，也不等待工具生命周期事件。

推荐最小 UI 状态：

```ts
type WidgetUiState =
  | "enabled" // 最新 assistant 消息里的 Widget，用户还未操作
  | "ready"   // 用户已经填写、选择或点击，具备提交值
  | "expired"; // 新 assistant 到来后，旧 Widget 全部过期
```

生命周期：

```text
assistant A arrives with Widget
-> enabled

user enters/selects/clicks
-> ready

user submits
-> 只提交这一轮已经就绪的 Widget 值，转换成普通 user message 发回 runtime

assistant B arrives
-> assistant A 中的所有 Widget 过期
```

同一轮 assistant 内容中无论包含多少个 Widget，前端都只应渲染一个提交按钮。点击提交时：

- 已就绪的 Widget 值被转换为普通 user message；
- 未就绪的 Widget 不随消息提交；
- 本轮 WidgetPanel 进入已提交状态，未就绪 Widget 变为过期、只读或不可操作；
- 后续新的 assistant 消息到来时，上一轮所有未提交 Widget 也应过期。

提交内容示例：

```text
格式: PDF
日期: 2026-05-28
确认删除 D:/tmp/a.txt: yes
```

不要把原始 Widget 标签发回 runtime。

### 6.8.6 写作要求

- 根据上下文动态生成控件，不要使用固定模板套话。
- 不需要结构化输入时，直接写自然语言问题，不要强行加控件。
- 危险操作需要确认时，使用 `confirm`，并在 `label` 或自然语言说明中写清文件名、动作和关键参数。
- `select` 的 `options` 必须用英文逗号分隔，不能把所有选项拼成一个无分隔符字符串。
- 控件标签可以与工具状态占位符共存，但不得写进 `EXEC` 行，也不得作为工具参数。

## 6.9 前端处理规则

- `record.role == "user"` 渲染为右侧用户气泡。
- `record.role == "assistant"` 渲染为左侧内容流，而不是厚重的整块气泡。
- 遇到 `[tool:status | call_id="..."]` 时，创建或更新工具状态组件。
- 遇到 Widget 标签时，按本文档的 Widget 控件标签协议渲染控件。
- 工具开始和结束记录不要作为普通 assistant 气泡展示。
- `gateway_message` 和 `tool` 记录不进入主聊天流；它们用于更新工具状态组件，或进入独立的调试和进度面板。
- Summary 类记录默认不展示为聊天内容。
- 使用 `payload.conversation_state` 作为 Agent 集群综合运行阶段。
- 只有 `conversation_state === "waiting"` 时允许输入和手动总结。
- `thinking` 或 `executing` 时允许请求暂停。
- 一轮完成必须满足：本轮先进入非 `waiting`，之后回到 `waiting`，并且已观测到的工具调用全部收敛。

## 6.10 推荐聊天渲染

前端推荐把对话渲染为两种不同视觉语义：

- 用户消息是明确的输入单元，使用右对齐紧凑气泡，最大宽度建议约为 `70%`。
- Assistant 消息是可阅读、可操作的内容画布，使用左对齐内容流，最大宽度建议约为 `760px`。

推荐组件结构：

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

视觉规则：

- 用户消息右对齐，使用明显气泡背景，例如品牌色或深色；只承载用户输入文本或 Widget 提交值。
- Assistant 消息左对齐，但不要包成厚重气泡；容器可以提供 avatar、间距、左侧竖线和最大宽度。
- Assistant 的段落、标题、列表、表格、代码块等应作为 Markdown 内容块独立渲染。
- 工具状态、Widget、代码块、表格可以使用轻量容器，但这些容器是内容块样式，不是新的聊天气泡。
- 工具状态不应弹窗或覆盖正文；应按 `[tool:status | call_id="..."]` 出现的位置渲染为
  inline timeline 或状态条。完成后可以折叠为一行摘要，例如 `已完成：ReadDocument 1.2s`。
- Widget 控件是 assistant 内容流的一部分，视觉上应像内嵌表单；提交后，前端把值转换为普通
  user 消息并展示为右侧用户气泡。

一句话原则：user 是气泡，assistant 是内容画布；工具是状态条，Widget 是内嵌控件；
所有内容按 `record.role` 和 `call_id` 归位。
