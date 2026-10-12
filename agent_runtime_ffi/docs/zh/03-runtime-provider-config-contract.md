# 3 Provider 配置加载契约

Provider 配置描述模型厂商、API key/base URL、模型定义和当前模型，不属于 Agent、Skill
或工具配置。

它是运行真实 Agent 宿主时的宿主注册平面之一：resources 定义产品暴露的资源，
provider config 定义 Runtime 可以调用哪个模型，agent cluster config 定义哪些具体
Agent 可以运行。

## 3.1 启动加载

宿主可在 start 前调用 `runtime.register_llm` 注册
`agent-runtime-llm-registration/v1`；该注册允许省略或为空，用于延迟模型配置。
`agent_runtime_create_v1` 不加载 provider 文件，也不接受 runtime config 路径。

推荐宿主通过 `payload.registration` 直接传入 JSON object。配置文件及其持久化位置属于
宿主实现细节，不应泄漏给 Runtime。`payload.input` 仍兼容 JSON object、JSON string 和
文件路径，供旧宿主迁移使用。

```json
{"schema":"agent-runtime-command/v1","type":"runtime.register_llm","payload":{"registration":{"schema":"agent-runtime-llm-registration/v1","id":"desktop-llm","providers":[],"current_model_uid":null}}}
```

## 3.2 运行期命令

- `runtime.register_llm`：start 前注册 `agent-runtime-llm-registration/v1`；可为空。
- `runtime.reload_llm`：使用与注册命令相同的 `payload.input` / `payload.registration`
  边界重新加载 LLM/provider；输入可以是 `agent-runtime-llm-registration/v1`，也可以是
  provider config JSON/file。
- `runtime.configure_providers`：推荐使用 `registration` JSON object 加载/覆盖 provider。
- `runtime.get_provider_definitions`：返回 `agent-runtime-provider-definitions/v1`。
- `runtime.set_current_model`：设置新会话使用的全局默认模型，要求 `model_uid` 为 uint32。
- `conversation.set_model`：按 `conversation_id` 设置已有会话的推理模型；前端的“当前模型”切换必须使用此命令，不能修改全局默认值。
- `runtime.set_auth_context`：设置宿主提供的认证上下文。

这些都是 `agent_runtime_invoke_v1` command，不存在独立 provider C 函数。

## 3.3 空模型 / 延迟模型配置

Provider 配置允许在首次启动时不存在，或显式为空：

```json
{"schema":"agent-runtime-llm-registration/v1","id":"desktop-llm","providers":[],"current_model_uid":null}
```

这是正式契约，不是兼容兜底。空模型状态下：

- Runtime 可以启动、注册 cluster、创建或恢复 conversation，并且
  `runtime.get_provider_definitions` 返回空 provider 列表。
- 任何需要调用 LLM 的 turn 都必须以可恢复的 provider/model 配置错误停止，直到宿主配置
  provider 并选择当前模型。
- 前端/宿主应表达“需要先配置模型与厂商”，不要随发布包写入示例 provider，也不要覆盖用户
  已有密钥。

宿主可在之后调用 `runtime.configure_providers` 或 `runtime.reload_llm` 加载真实 provider，
如果加载内容没有有效 `current_model_uid`，再调用 `runtime.set_current_model` 选择模型。
非空 `current_model_uid` 必须引用已加载 provider 的 enabled model。它只作为新会话没有
显式模型时的默认值，不会覆盖已有 conversation 的模型。已有会话应调用
`conversation.set_model` 显式切换；cluster 和 Agent 配置不得为了延迟配置而内嵌 provider
密钥或虚假模型占位。

不要用示例/占位 provider 无条件覆盖已正确加载的配置。生产密钥由宿主管理；错误通过
`agent-runtime-result/v1` 和同线程 `agent_runtime_last_error_json_v1` 返回。

## 3.4 按 API 范式配置模型推理

仍只支持 `openai_chat_completions`、`openai_responses`、`anthropic_messages` 三种兼容接口。推理参数按所选范式编码，不按厂商、URL 或模型名称增加专用适配规则。

在 `enabled_models[]` 中增加可选 `reasoning` 对象；旧 providers/models 配置在 `models[]` 中使用相同对象。`runtime.get_provider_definitions` 的每个模型返回该配置，持久化和宿主显式重载保留它。省略 `reasoning` 或使用 `{}` 不新增任何请求字段，不改变已有行为。

| 配置 | 可用范式 | 上游字段 |
|---|---|---|
| `effort` | Chat | `reasoning_effort` |
| `effort` | Responses | `reasoning.effort` |
| `summary` | Responses | `reasoning.summary` |
| `effort` | Anthropic | `output_config.effort` |
| `thinking` | Anthropic | `thinking.type` |
| `budget_tokens` | Anthropic | `thinking.budget_tokens` |

`effort` 可取 `none/minimal/low/medium/high/xhigh/max`；Anthropic 不接受 `none/minimal`，关闭思考使用 `thinking: "disabled"`。`summary` 可取 `auto/concise/detailed`。`thinking` 可取 `disabled/enabled/adaptive`，独立于 effort，不指定时由兼容服务决定。固定预算 `enabled` 必须同时提供至少 1024 的 `budget_tokens`；其他模式禁止携带预算。

### Responses 示例

```json
{
  "schema": "agent-runtime-llm-registration/v1",
  "id": "desktop-llm",
  "providers": [{
    "id": 1,
    "name": "Compatible service",
    "type": "openai-gpt",
    "api_key": "HOST_SUPPLIED",
    "base_url": "https://example.invalid/v1",
    "api_paradigm": "openai_responses",
    "enabled_models": [{
      "uid": 1001,
      "model_id": "host-model",
      "max_context_tokens": 128000,
      "reasoning": {"effort": "high", "summary": "auto"}
    }]
  }]
}
```

Chat 将模型配置改为 `"reasoning": {"effort": "high"}`。Anthropic 可配置 `{"effort": "high", "thinking": "adaptive"}`，或固定预算 `{"thinking": "enabled", "budget_tokens": 2048}`。这是请求配置，不是 system 提示词，也不是 Runtime 的 thinking 状态或思考轮数。

### 请求行为与校验

- 注册/加载时检查字段与范式是否匹配；非法枚举、错误字段、矛盾模式/预算明确失败。
- 流式、非流式、普通消息、原生工具调用及重试沿用同一模型配置。按模型 UID 显式传递，两个同名模型不会互相借用配置。
- Chat 显式配置 effort 时，调用方传入的输出上限编码为 `max_completion_tokens`；Responses 使用 `max_output_tokens`；Anthropic 使用 `max_tokens`。
- 显式启用推理的 Chat/Responses effort（非 none）或 Anthropic enabled/adaptive 模式，不发送 `temperature/top_p`，避免兼容接口拒绝组合。未配置或显式关闭时保留原有采样行为。
- Anthropic 固定预算必须小于本次请求的 `max_tokens`（没有指定时现有默认值为 4096）；不自动提高输出预算。启用思考与强制 tool/any 选择冲突时在发送前报配置错误，不默默取消强制执行要求。
- Anthropic thinking/redacted_thinking 块和签名完整保存于既有 `provider_items`、历史及 ledger，并在后续 Anthropic 请求中回传；思考内容不作为可执行文本或用户回答输出。切换到 Responses 时不发送 Anthropic thinking 块。
- 不保证每个兼容服务/模型都支持所有枚举；真实支持范围由目标端点决定。上游拒绝参数时保留错误，不自动去掉配置重试或声称已生效。

Lit 模型编辑器按范式显示推理强度及附加字段；“服务端默认”省略参数。编辑或添加提供商时保留其他模型已有的 reasoning。C ABI 和命令 schema 不变；SDK 沿用 JSON 注册/配置边界，无需新增 C 函数。

参考：[OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning)、[Claude effort](https://platform.claude.com/docs/en/build-with-claude/effort)、[Claude extended thinking](https://platform.claude.com/docs/en/build-with-claude/extended-thinking)。
