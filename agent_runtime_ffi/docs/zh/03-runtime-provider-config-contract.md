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
- `runtime.set_current_model`：要求 `model_uid` 为 uint32。
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
非空 `current_model_uid` 必须引用已加载 provider 的 enabled model。已有 conversation 的后续
LLM 调用使用新的当前模型；cluster 和 Agent 配置不得为了延迟配置而内嵌 provider 密钥或
虚假模型占位。

不要用示例/占位 provider 无条件覆盖已正确加载的配置。生产密钥由宿主管理；错误通过
`agent-runtime-result/v1` 和同线程 `agent_runtime_last_error_json_v1` 返回。
