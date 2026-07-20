# 3 03 EXEC 行式协议与工具执行

模型通过正文中的行式协议调用工具，不使用供应商 Function Calling：

```text
EXEC ToolName --arg value
```

parser 支持引号、多行变量和 heredoc；`tool_runner` 将命令拆成工具名与原始参数文本，
再交给 corework `DynamicSystem`。

## 3.1 可调用条件

工具必须同时满足：

1. 已注册为 local system 或 runtime dynamic/RPC system；
2. 名称存在于当前 Agent 合并后的 `ACTIVE_TOOLS`；
3. 当前 Skill/state/tool filter 允许；
4. 参数名符合 `AISystemMetadata` 或 `RuntimeToolMetadata`。

因此“Runtime 已注册”不等于“所有 Agent 可调用”。Skill 的 `tools` 是权限白名单。

## 3.2 执行结果

统一结果为 `AIOutput`：

```json
{"result": {}, "to_ai": "模型下一轮必须知道的完整结果", "error_code": 0}
```

- `result` 给程序和宿主保留结构化数据。
- `to_ai` 作为工具结果进入模型上下文，必须完整且非空。
- 失败也要返回可行动的 `to_ai`，包括原因、限制和允许的下一步。
- 强顺序工具可以在 `to_ai` 中明确“下面调用 X 完成 Y”，但 X 必须也在 Skill 白名单中。

查表类工具应在 `to_ai` 中保留关键行、列名、标识和分页事实，而不是只写“查询成功”。

## 3.3 并发与展示命令

`executing` 可并发执行同一决策中的独立工具调用。Runtime 分开保存真实执行命令和
display command，使变量展开后的参数用于执行，原始表达式用于日志/前端展示。
工具完成后结果回灌，再进入 thinking；不会把供应商要求成对的原生 tool role 历史
直接复用。

## 3.4 Local 工具边界

普通 Agent 当前可由 Skill 引用的 local operation 分为：公共工具、渐进 Skill、Plan、
Agent 协作、后台任务和按 Agent 路由的 `RagRetrieve`。Workflow Editor 与 Agent Test
使用各自隔离 role Skill 的专属工具。完整清单见
`examples/guides/zh/06-builtin-tools-and-agents.md`。

ledger、prompt、Skill loader、Draft 等 `buns_system` 是状态机内部系统，不应仅因存在
于 registry 就写进业务 Skill。

下一篇：[04 技能系统与提示词](04_skill_system_and_prompts.md)
