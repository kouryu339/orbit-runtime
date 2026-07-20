# 8 Connect External RAG

External RAG is a host-provided knowledge service. Runtime decides when to
retrieve, routes through the current Agent's endpoint, and injects `to_ai` into
dynamic context. The host owns ingestion, indexing, authorization, retrieval
quality, and service lifecycle.

Register each knowledge service in the resource registration:

```json
{
  "schema": "agent-runtime-resource-registration/v1",
  "id": "service-resources",
  "rpc_endpoints": [{
    "id": "product-knowledge",
    "protocol": "json-lines",
    "endpoint": "127.0.0.1:51001",
    "timeout_ms": 30000
  }]
}
```

Then bind an Agent profile, or override one concrete `cluster.agents[]`
instance, with `retrieval.endpoint_id`. Retrieval belongs to the Agent; legacy
cluster-root and `runtime.retrieval` configuration is rejected.

```json
{
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

The endpoint is a dedicated JSON Lines TCP service, not a business Agent Tool
endpoint. Runtime opens one connection per request, writes one UTF-8 JSON line,
closes the write side, and reads the first response line.

```json
{
  "type": "retrieval",
  "request": {
    "tool_name": "RagRetrieve",
    "conversation_id": "conv-123",
    "args_json": {
      "query": "What is the return policy?",
      "profiles": ["order_policy"],
      "top_k": 5,
      "score_threshold": 0.3
    }
  }
}
```

Return exactly one response line:

```json
{
  "type": "retrieval_output",
  "output": {
    "error_code": 0,
    "result": {"hits": [{"source": "return-policy.md", "score": 0.86}]},
    "to_ai": "[1] source=return-policy.md score=0.860\nCustomers may return..."
  }
}
```

Retrieval success is `error_code: 0`. No hits is also success with an empty
`to_ai`. A nonzero code is handled according to the Agent's `fail_policy`.

`to_ai` is the model-facing knowledge body. Include useful chunk text, source,
version or section, score, applicability, and clear separators. Do not leave all
useful content only in `result`. Keep responses bounded and deduplicated, and
treat retrieved documents as untrusted reference material rather than
instructions.

Automatic retrieval runs before the first thinking step for each user turn and
does not require `RagRetrieve` in a Skill. To allow explicit second-pass
retrieval, list `RagRetrieve` in the relevant role or feature Skill. The Agent
remains bound to its endpoint, but may supply `profiles`, `top_k`, and
`score_threshold`.

The wire request carries a Runtime `conversation_id`, not trusted tenant/user
identity. The backend must enforce profile/collection authorization, cap query
length, `top_k`, timeout, and response size, and never treat model-provided
profiles as authorization. Maintain a trusted conversation-to-tenant mapping or
use separate endpoints for separate security domains.

The current JSON Lines transport is plain TCP without built-in TLS or
authentication. Keep it on loopback or a controlled private/service-mesh
network; do not expose it directly to the public internet.

See [`examples/qiyunshanyoucha`](../../qiyunshanyoucha) for a runnable local
server, index, and resource registration.

