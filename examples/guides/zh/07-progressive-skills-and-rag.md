# 7 渐进式 Skill 与 Agent RAG

Skill 和知识检索解决两种不同的上下文膨胀：

- 渐进式 Skill：只在需要时加载能力说明和工具白名单。
- Agent RAG：只把与当前问题相关的外部知识放进上下文。

## 7.1 渐进式 Skill 加载

Role Skill 是 Agent 的稳定身份，放在 `MAIN_SKILLS` 中，不被动态切换。Feature
Skill 可以由 Agent 在运行时按需激活。

系统 thinking Skill 默认提供：

| 工具 | 作用 |
|---|---|
| `GetSkillsList` | 返回可用 feature Skills、描述和当前激活状态。 |
| `UpdateSkills` | 全量替换当前导入的 feature Skills，并重新计算工具白名单。 |

Role Skill 应告诉 Agent 何时加载哪个能力：

```markdown
当用户要求处理 Excel 文件时：

1. 若不确定能力名，调用 `GetSkillsList`。
2. 调用 `UpdateSkills --skills excel` 激活 Excel feature。
3. 再按 Excel Skill 的说明调用其工具。
```

`UpdateSkills` 是替换语义，不是追加语义。若希望同时保留 `excel` 和 `fileops`，
必须一次传入两者：

```text
UpdateSkills --skills excel,fileops
```

调用后 Runtime 会更新 `IMPORTED_SKILLS`，加载 Skill 正文，并用 main + imported
Skills 的 `tools` 重建 `ACTIVE_TOOLS`。被移除 Skill 的工具会立即离开白名单。

Feature 的 `description` 要写清触发条件，因为 Agent 在加载正文前主要依靠名称和
描述选择 Skill。不要把所有 feature 固定塞进 profile；否则渐进式加载失去意义。

## 7.2 注册多个知识库端点

resources 只登记知识库连接，不决定哪个 Agent 使用哪个库：

```json
{
  "rpc_endpoints": [{
    "id": "product-knowledge",
    "protocol": "json-lines",
    "endpoint": "127.0.0.1:51001",
    "timeout_ms": 30000
  }, {
    "id": "policy-knowledge",
    "protocol": "json-lines",
    "endpoint": "127.0.0.1:51002",
    "timeout_ms": 30000
  }]
}
```

Retrieval endpoint 是 Runtime 的专用知识检索边界，当前使用 `json-lines`。普通业务
Tools 仍应优先使用 Agent Tool SDK 和 gRPC，不要因为 RAG 链接使用 json-lines
就手写业务 Tool 协议。

## 7.3 给每个 Agent 配置 RAG

Retrieval 属于 Agent。可以写在 resource profile 中作为该类 Agent 的默认策略：

```json
{
  "id": "commerce.product_advisor",
  "name": "Product Advisor",
  "role": "product_advisor",
  "retrieval": {
    "enabled": true,
    "mode": "before_thinking",
    "trigger": "first_thinking_per_user_turn",
    "tool_name": "RagRetrieve",
    "endpoint_id": "product-knowledge",
    "profiles": ["catalog", "product_manual"],
    "top_k": 5,
    "score_threshold": 0.3,
    "fail_policy": "soft",
    "inject_as": "dynamic_context"
  }
}
```

也可以在 `cluster.agents[].retrieval` 中覆盖某个实例：

```json
{
  "id": "policy-agent-cn",
  "profile": "commerce.policy_advisor",
  "retrieval": {
    "enabled": true,
    "endpoint_id": "policy-knowledge",
    "profiles": ["policy_cn"],
    "top_k": 8
  }
}
```

实例配置覆盖 profile 默认值。`runtime.retrieval` 和 cluster 顶层 `retrieval` 已废弃；
同一个 cluster 中的不同 Agent 可以选择 resources 中不同的 endpoint。

关键字段：

| 字段 | 作用 |
|---|---|
| `endpoint_id` | 引用 resources 中已注册的知识库端点。 |
| `profiles` | 该 Agent 默认检索的知识域/索引 profile。 |
| `top_k` | 默认返回条数。 |
| `score_threshold` | 默认相关度阈值。 |
| `fail_policy` | `soft` 忽略检索失败继续；`hard` 中止当前推理。 |
| `tool_name` | 显式二次召回工具名，默认 `RagRetrieve`。 |

## 7.4 默认召回与二次召回

启用 retrieval 后，每个用户回合第一次 thinking 前，Runtime 用用户问题自动查询
当前 Agent 的 endpoint，并把结果注入 dynamic context。这条默认召回不要求
`RagRetrieve` 出现在业务 Skill 中。

如果希望 Agent 在思考后改写更精确的 query 再查一次，需要在对应 role/feature
Skill 中显式加入：

```yaml
tools: ["RagRetrieve"]
```

此时 Agent 会看到 `query`、`profiles`、`top_k`、`score_threshold` 参数。未显式
传入的可选参数继承该 Agent 的 retrieval 配置。二次召回仍只能访问该 Agent 绑定
的 endpoint，不能借工具名越权切换到其他知识库。

Role Skill 还应说明何时二次召回，例如：

```markdown
默认知识不足、用户问题包含具体条款编号，或需要缩小到某一产品型号时，调用
`RagRetrieve` 发起更精确的 query。已有上下文足够时不要重复召回。
```

RAG 返回内容是参考资料，不是高权限指令。Agent 应核对相关性，不足时说明缺口，
不要根据检索片段编造结论。

宿主知识库服务的资源注册、JSON Lines 请求/响应、`to_ai` 编写和权限边界见
[`接入外部 RAG`](08-external-rag.md)。
