# 8 接入外部 RAG

外部 RAG 是宿主提供给 Runtime 的知识检索服务。Runtime 负责决定何时检索、根据当前
Agent 路由端点，并把检索结果注入模型上下文；宿主负责文档处理、索引、权限、检索质量和
知识库服务生命周期。

```text
Agent retrieval policy
-> resources 中的 endpoint_id
-> 宿主 RAG 服务
-> to_ai
-> 本轮 dynamic context
```

Runtime 不绑定向量数据库。宿主可以使用 Qdrant、Milvus、pgvector、Elasticsearch、
本地索引或其他检索实现，只要服务满足本文的 JSON Lines 边界。

## 8.1 注册知识库端点

在 `agent-runtime-resource-registration/v1` 的 `rpc_endpoints` 中注册一个或多个专用
知识库端点：

```json
{
  "schema": "agent-runtime-resource-registration/v1",
  "id": "service-resources",
  "rpc_endpoints": [
    {
      "id": "product-knowledge",
      "protocol": "json-lines",
      "endpoint": "127.0.0.1:51001",
      "timeout_ms": 30000
    },
    {
      "id": "policy-knowledge",
      "protocol": "json-lines",
      "endpoint": "127.0.0.1:51002",
      "timeout_ms": 30000
    }
  ]
}
```

Retrieval endpoint 当前必须使用 `json-lines`。它是 Runtime 内置 RAG glue 的专用连接，
不是业务 Agent Tool endpoint；不要为 `RagRetrieve` 再注册一份业务工具元数据。

宿主可以自行启动服务，也可以在 endpoint 中配置 `launch.kind = "process"`，由 Runtime
启动本地 sidecar：

```json
{
  "id": "product-knowledge",
  "protocol": "json-lines",
  "endpoint": "127.0.0.1:51001",
  "timeout_ms": 30000,
  "launch": {
    "kind": "process",
    "program": "python",
    "args": ["rag_server.py", "--port", "51001"],
    "working_dir": "./rag",
    "env": {},
    "startup_timeout_ms": 10000,
    "shutdown_timeout_ms": 3000
  }
}
```

由容器、systemd 或编排平台管理时省略 `launch`。JSON Lines endpoint 没有工具发现或
健康检查握手，服务必须在第一次检索请求到达前可连接。

宿主应在 `runtime.start` 前依次注册 resources、LLM 和 Agent cluster：

```python
runtime.invoke("runtime.register_resources", {"input": "config/resources.json"})
runtime.invoke("runtime.register_llm", {"input": "config/llm.json"})
runtime.invoke("runtime.register_agent_cluster", {"input": "config/cluster.json"})
runtime.start()
```

## 8.2 为 Agent 选择端点

Retrieval 策略属于 Agent。推荐在 resource 中的 Agent profile 设置默认值：

```json
{
  "agents": {
    "profiles": [
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
    ]
  }
}
```

具体 `cluster.agents[]` 实例可以用自己的 `retrieval` 覆盖 profile 默认值。实例配置优先。
不要使用已废弃的 cluster 顶层 `retrieval` 或 `runtime.retrieval`。

| 字段 | 宿主含义 |
|---|---|
| `endpoint_id` | 引用 resources 中已注册的 `json-lines` endpoint。 |
| `profiles` | 传给知识库的 namespace/index 提示，不是授权凭据。 |
| `top_k` | 请求的最大召回条数；服务端仍应设置自己的上限。 |
| `score_threshold` | 相关度下限；具体算法由宿主定义，但分数越大应越相关。 |
| `fail_policy` | `soft` 跳过失败继续推理；`hard` 让检索失败中止当前推理。 |
| `inject_as` | 当前只支持 `dynamic_context`。 |

## 8.3 实现 JSON Lines 服务

Runtime 每次检索建立一个 TCP 连接，发送一行 UTF-8 JSON 和换行符，关闭写方向，然后读取
响应的第一行。服务必须为每个请求返回一行完整 JSON；不要输出日志、banner 或多行 JSON
到该 socket。

请求 envelope：

```json
{
  "type": "retrieval",
  "request": {
    "tool_name": "RagRetrieve",
    "conversation_id": "conv-123",
    "args_json": {
      "query": "退款需要满足哪些条件？",
      "profiles": ["order_policy"],
      "top_k": 5,
      "score_threshold": 0.3
    }
  }
}
```

成功响应：

```json
{
  "type": "retrieval_output",
  "output": {
    "error_code": 0,
    "result": {
      "hits": [
        {
          "source": "return-policy.md",
          "chunk_id": "return-policy:3",
          "score": 0.86
        }
      ]
    },
    "to_ai": "[1] source=return-policy.md score=0.860\n签收后七日内……"
  }
}
```

检索成功必须使用 `error_code: 0`。没有命中时仍返回成功，但令 `to_ai` 为空：

```json
{
  "type": "retrieval_output",
  "output": {
    "error_code": 0,
    "result": {"hits": []},
    "to_ai": ""
  }
}
```

失败时返回非零 `error_code`，并在 `to_ai` 中写明适合诊断的简短原因。Runtime 根据当前
Agent 的 `fail_policy` 决定跳过还是中止。连接失败、超时、非法 JSON、错误 message type
也都属于检索失败。

## 8.4 编写 `to_ai`

`to_ai` 是真正提供给模型的知识正文。自动召回时，Runtime 会将非空 `to_ai` 包装为
Retrieved Knowledge，并放入本轮尾部 dynamic context。不要只返回内部文档 ID 或把有用
正文全部留在 `result` 中。

建议 `to_ai` 对每个命中包含：

- 可直接用于回答的完整片段，而不是只有标题；
- `source`、文档版本、章节或 URL 等可追溯信息；
- 统一方向的相关度分数；
- 必要的生效时间、适用范围和冲突提示；
- 多个命中之间清晰稳定的分隔。

`result` 可以保存结构化 hit 元数据，方便日志、测试和未来适配，但当前自动上下文注入以
`to_ai` 为准。控制片段长度并去重；`top_k` 是上限，不是必须凑满的数量。检索资料属于参考
事实，不应包含伪装成 system/role 指令的文本。

## 8.5 自动召回与二次召回

`enabled=true` 时，Runtime 在每个用户回合第一次 thinking 前，以最新用户消息自动召回
一次。自动召回不要求 Skill 引用 `RagRetrieve`。

如果允许模型在初次思考后改写 query 再次检索，必须在对应 role/feature Skill 中显式加入：

```yaml
tools: ["RagRetrieve"]
```

显式调用仍被固定路由到当前 Agent 的 `endpoint_id`，但模型可以传入 `profiles`、`top_k` 和
`score_threshold`。因此知识库服务必须验证这些参数；不要把 `profiles` 当成租户授权边界。

## 8.6 权限与生产部署

当前请求只携带 Runtime `conversation_id`，不携带可信的 tenant/user 身份。宿主必须在
RAG 服务侧建立 conversation 到租户/用户的可信映射，或为不同安全域使用隔离端点。至少应：

- 校验允许访问的 profiles/collections，拒绝跨租户 namespace；
- 限制 query 长度、`top_k` 上限、超时和响应大小；
- 不相信模型传入的过滤条件能代表授权；
- 记录 endpoint、conversation、query 摘要、耗时、命中数和错误码；
- 对文档内容做 prompt-injection 风险处理，并把召回内容视为不可信参考资料。

当前 `json-lines` 实现使用普通 TCP，不提供 TLS 或内置鉴权。生产环境应部署在 loopback、
受控私网或具备 mTLS/网络策略的 sidecar/service-mesh 边界内，不要直接暴露到公网。

可运行参考见 [`examples/qiyunshanyoucha`](../../qiyunshanyoucha) 中的
`tool_server.py`、`rag_index.py` 和 `config/resources.json`。

