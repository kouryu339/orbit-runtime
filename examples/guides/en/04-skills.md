# 4 Write Skills

A Skill defines an agent's role and working method. It does not implement a
tool. A project normally provides one role Skill and optional feature Skills:

```text
skills/
  role/order_admin/SKILL.md
  feature/after_sales/SKILL.md
```

Register this root through `skills.root_dir` in the resource registration.

```markdown
---
name: order_admin
description: "Handles order lookup and after-sales requests."
kind: role
tools: ["OrderList", "OrderGet", "ReturnCreate"]
---

# Order service role

- Call `OrderList` when the user does not know the order id.
- Confirm the target before changing business state.
- Never invent order, user, or refund status.
```

`name` is the stable Skill reference, `kind` is `role` or `feature`, and
`tools` lists already registered tools that this Skill exposes to the AI.
Declaring a name in `tools` does not implement or register that tool.

This is an enforced Runtime allowlist, not merely a prompting hint. Tools that
active Skills do not reference are omitted from the agent's tool context and
rejected if the agent constructs a call directly. If one tool's `to_ai` result
recommends a follow-up tool, that follow-up must also be listed by an active
Skill. Keep strongly ordered tool chains in one role/feature Skill and document
their prerequisites, order, and stopping conditions.

Bind Skills to a reusable agent profile in resources:

```json
{
  "agents": {
    "profiles": [{
      "id": "commerce.order_admin",
      "name": "Order Admin",
      "role": "order_admin",
      "features": ["after_sales"]
    }]
  }
}
```

Create concrete instances from that profile in an agent cluster. The profile
identifies the agent type; `agents[].id` identifies one concrete instance.
Collaboration, delegation, and reporting tools should be granted explicitly by
the role Skill that knows when to use them.

See the full
[`Runtime Skill Authoring Guide`](../../../agent_runtime_ffi/docs/en/10-runtime-skill-authoring-guide.md).
