# Workflow 资源对外接口

当前只开放六项资源操作。AI 工具和 Runtime 命令一一对应，不再提供创建草稿、读取、写入、编译、单独测试、脚本执行或分步运行接口。

| 操作 | AI 工具 | Runtime 命令 | 参数 |
|---|---|---|---|
| 列表 | `listWorkflows` | `workflow.list` | `kind?`：`all`（默认）、`draft`、`registered` |
| 注册 | `registerWorkflow` | `workflow.register` | `source: string`，完整资源源文本 |
| 移除 | `removeWorkflow` | `workflow.remove` | AI: `workflow_id, expected_revision`；Runtime: `id, expected_revision` |
| 验证 | `validateWorkflow` | `workflow.validate` | `source: string`，完整资源源文本 |
| 转正 | `promoteWorkflow` | `workflow.promote` | AI: `workflow_id, expected_revision`；Runtime: `id, expected_revision` |
| 执行 | `executeWorkflow` | `workflow.execute` | AI: `workflow_id, input.<name>?, trace?`；Runtime: `id, inputs?, trace?` |

执行还支持宿主身份上下文 `conversation_id, agent_id, turn_id`。前两个必须同时提供且非空。AI 通过自身上下文传递身份。真实执行继续由 AI 工具权限策略控制，验证不执行节点、不申请执行授权。

## 对外提供的 Schema 与语法说明

Runtime 包公开提供以下文件，宿主分发时应连同 DLL 一起保留；这些文件不依赖开发仓库路径：

| 包内路径 | 用途 |
|---|---|
| `schemas/resource.schema.json` | 完整资源结构，包含内嵌的逻辑 Schema |
| `schemas/logic.schema.json` | 单独的 logic 结构 |
| `docs/zh/workflow-json-source-syntax.md` | 中文源文本基本语法与最小示例 |
| `docs/en/workflow-json-source-syntax.md` | 英文源文本基本语法与最小示例 |

仓库源文件为 `corework/src/workflow/resource.schema.json`、`corework/src/workflow/logic.schema.json`，语法说明源文件为 `ai-assistant/prompts/{zh,en}/workflow_tool_rules.md`。两个 Schema 使用 JSON Schema Draft 2020-12；`$id` 分别为 `urn:orbit:workflow:resource:v1`、`urn:orbit:workflow:logic:v1`，均不是下载地址。资源 Schema 已内嵌逻辑定义，校验时不需要联网获取。

Schema 校验的是 Runtime 解析、规范化后的结构，不能直接解析含 `input.name` 等裸表达式的源文本。宿主校验作者提交的源文本时调用 `workflow.validate`；只通过 Schema 不代表工具契约、引用或作用域有效。当前没有读取 Schema 的 Runtime 命令或 AI 工具，文件通过 Runtime 包对外分发；thinking-pro 内嵌的简短说明用于让 AI 直接掌握基本语法。

## JSON 资源文档

```text
{
  "schema": "orbit.workflow.resource/v1",
  "id": "echo-name",
  "metadata": {
    "name": "姓名回显",
    "description": "返回输入姓名。",
    "extensions": {}
  },
  "state": "draft",
  "revision": 1,
  "logic": {
    "schema": "orbit.workflow.logic/v1",
    "inputs": {
      "name": {"type": "String", "disablename": "姓名", "description": "待返回的姓名。"}
    },
    "variables": {},
    "steps": [],
    "returns": {
      "name": {"value": input.name, "disablename": "姓名", "description": "原样返回输入姓名。"}
    }
  }
}
```

此例使用之前约定的 JSON 形状源语法：`returns.name.value` 是裸引用。整份文档必须作为 `source` 字符串交给 Runtime 解析；字符串 `"input.name"` 始终是文字。

- `state` 必填，取 `draft` 或 `registered`。首次注册按文档声明加载对应状态。已有身份不能通过再次注册切换状态；草稿转正使用 promote，正式资源不允许直接降为草稿。
- 新身份的 `revision` 必须为 1。同一身份再次注册表示整份替换，文档 revision 必须等于当前值；提交成功后 Runtime 递增并返回新值。不存在另外的 Write/Update 工具。
- `metadata.extensions` 按宿主命名空间保存，注册、转正和落盘后完整保留。
- 注册、验证共用编译、工具契约、作用域和正式引用检查。注册失败不覆盖旧版本；验证不保存、不会改变 revision，也不会运行节点。
- 执行按已接受的资源 state 选择草稿或正式路径，不需要 `mode` 或额外的 Draft 工具。草稿执行仍是真实执行，不是静态验证。
- 移除和转正必须携带当前 `expected_revision`。被正式工作流引用的资源不能直接移除；破坏已有调用方接口的更新也会被拒绝。
- Runtime 持久化源文本，启动时在工具契约安装后自动恢复草稿和正式资源，按引用依赖编译。源文件中的 revision 原样恢复，加载不会写文件或执行节点。
- 新资源保存为管理目录下的 `<id>.workflow.json`；也可以在启动前放入完整 `.workflow` 或 `.json` 资源。旧脚本及旧图资源跳过并记录日志，不自动迁移。
- 移除会删除管理目录中的对应源文件，包括手动导入的文件；目录外的作者备份不受影响。核心编译器和执行器继续沿用原机制。

## Runtime 请求

注册和验证的 `source` 是整份文档的字符串。这样节点 args 和 returns.value 可以包含 `input.name`、`1.session`、`add(var.count, 1)` 等裸表达式，同时外层命令仍为正常 JSON。

```json
{"type":"workflow.validate","payload":{"source":"<完整资源源文本>"}}
```

```json
{"type":"workflow.register","payload":{"source":"<完整资源源文本>"}}
```

```json
{"type":"workflow.promote","payload":{"id":"echo-name","expected_revision":1}}
```

```json
{"type":"workflow.execute","payload":{"id":"echo-name","inputs":{"name":"Ada"},"trace":false}}
```

```json
{"type":"workflow.remove","payload":{"id":"echo-name","expected_revision":2}}
```

注册和验证返回简要身份、state、revision、metadata、输入输出契约和 valid/committed；转正返回当前资源；执行返回输出和可选 trace。旧命令既不出现在 capabilities / 工具描述中，直接调用也返回 `deprecated_api`。旧 register 的 id-only 载荷不再接受。

## 独立页面弃用

Workflow Studio 和 Agent Test Studio 两个独立页面已弃用；studio.open_workflow 和 studio.open_agent_test 不再列入 capabilities，直接调用返回 deprecated_api。页面实现暂时保留为内部兼容代码，本阶段不迁移画布或测试页。

## 列表

`workflow.list` / `listWorkflows` 恢复正常提供，只读返回资源摘要（id、名称、描述、kind、revision、验证状态），不返回逻辑或脚本全文。`kind` 省略或为 null/all 时列出全部，draft/registered 按状态筛选；非法筛选报错，不执行节点。
