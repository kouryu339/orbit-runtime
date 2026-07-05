# 7 Progressive Skills and Per-Agent RAG

Role Skills remain stable. Feature Skills can be loaded progressively with
`GetSkillsList` and `UpdateSkills`. `UpdateSkills` replaces, rather than appends
to, the imported feature set and rebuilds the active tool allowlist from main
plus imported Skills.

Resources may register multiple dedicated knowledge endpoints. Retrieval policy
belongs to an agent profile or a concrete `cluster.agents[]` entry, not to the
runtime or cluster root. An instance retrieval block overrides its profile:

```json
{
  "id": "product-agent-1",
  "profile": "commerce.product_advisor",
  "retrieval": {
    "enabled": true,
    "endpoint_id": "product-knowledge",
    "profiles": ["catalog", "product_manual"],
    "top_k": 5,
    "score_threshold": 0.3
  }
}
```

Before the first thinking step of a user turn, Runtime automatically retrieves
from the current agent's endpoint and injects relevant context. For explicit
second-pass retrieval, add `RagRetrieve` to that agent's active role/feature
Skill. Optional arguments inherit the agent configuration, and routing remains
bound to the configured endpoint.

Retrieval endpoints currently use the dedicated `json-lines` boundary. Normal
business tools should still use the Agent Tool SDK and gRPC.

See the [Chinese guide](../zh/07-progressive-skills-and-rag.md) for full endpoint,
profile, progressive-loading, and second-recall examples.

See [Connect External RAG](08-external-rag.md) for the host service wire contract,
`to_ai` guidance, lifecycle, and authorization boundary.
