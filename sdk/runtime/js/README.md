# Orbit Runtime 浏览器 SDK

JS／TS 通过独立 Web Worker 加载 Rust WASM。Agent 状态机、工具执行、Workflow 编译器和执行器共用 Native 实现；Native SDK 继续使用 DLL／SO。本包不依赖 Lit 或 Svelte。

0.5.3 是首个浏览器实现，通过 GitHub Release 提供 ZIP 和可安装的 `.tgz`，没有发布到 npm 公共仓库。适用 HTTPS 或 localhost；默认使用 IndexedDB 和 Web Locks。`storage: 'memory'` 明确选择临时资源，不会在存储故障时自动降级。

完整接入规范、部署要求和能力限制见 [USAGE.zh-CN.md](./USAGE.zh-CN.md)。可运行示例位于 [examples/quickstart.js](./examples/quickstart.js)。

```ts
import { createRuntime } from '@orbit-runtime/browser';

const runtime = await createRuntime({
  namespace: 'my-project',
  skills: [roleSkillMarkdown],
  llm: llmRegistration,
  clusters: [clusterRegistration],
  endpoints: [{ id: 'host-tools', protocol: 'host' }],
  async host(operation, payload, { signal }) {
    if (operation === 'rpc.list_tools') return toolCatalog;
    if (operation === 'rpc.execute') {
      // Dispatch to your browser tools, WebSocket bridge or backend.
      return executeTool(payload.request, { signal });
    }
    throw new Error(`Unsupported host operation: ${operation}`);
  },
});

const unsubscribe = runtime.subscribe(event => renderEvent(event));
const { conversation_id } = await runtime.invoke('conversation.spawn', {
  cluster_id: 'my-team',
});
await runtime.invoke('conversation.send_message', {
  conversation_id, content: '开始处理',
});
// Later: unsubscribe(); await runtime.dispose();
```

`skills` 是 SKILL.md 正文，`workflows` 是完整资源源文本数组；不接受文件路径。LLM 配置采用现有 `agent-runtime-llm-registration/v1`，API Key 只驻留内存，不写入资源存储。浏览器网络请求受 CORS 约束；可以配置宿主自己的模型代理。

集群使用 `agent-runtime-agent-cluster-registration/v1`，至少一个 Agent 且每个 Agent 绑定一个 Role Skill。首版字段支持：集群 `id/description/focus_agent_id/agents/max_thinking_rounds/permissions`，Agent `id/name/role/features/model_uid/system_skills/tool_protocol`。不支持的字段会报错。注册在启动前完成。

宿主工具发现返回 `{schema:'corework-agent-tool/v1', tools:[...]}`，工具字段和参数沿用 Native 元数据。执行返回 `{result, to_ai, error_code}`。等待操作有期限；暂停、批准、关闭可并发处理；宿主通过 `AbortSignal` 处理取消。

工作流通过 `runtime.invoke('workflow.register', {source})` 提交，源文本可使用现有 Pure／引用语法。`workflow.validate/list/promote/remove/execute` 使用相同语义。写入先经过 Runtime 校验，再等待存储提交；失败回滚目录和源文本，不发出成功变更事件。

同一个资源命名空间只能存在一个 Runtime。浏览器首版存在存活 Conversation 时拒绝工作流写操作；AI 侧不注册工作流写入工具。这样不会绕过异步存储提交。完整的会话重载恢复、Skill／集群内容热更新、图片导入、原生文件、原生进程和原生 gRPC 尚未接入；以 `runtime.capabilities` 为准。重建 Runtime 会恢复资源，LLM 配置和宿主工具需要重新提供。

Lit 和 Svelte 都可使用 `@agent-runtime/conversation-element/transport` 导出的 `WorkerConversationTransport`：

```ts
const transport = new WorkerConversationTransport(runtime);
// Set the same transport on either Conversation component.
```

部署时保留 `dist/` 的相对目录：SDK JS、Worker、绑定 JS 和 `.wasm`。WASM 文件使用 `application/wasm` MIME。可通过 `workerUrl/wasmModuleUrl` 指定资源地址；打包器应将动态 Worker 及 WASM 作为独立资源输出。服务地址发生变化不应改变宿主的资源命名空间。

公开 Schema 位于 `dist/schemas/`，也通过 `workflowSchemas.logic/resource` 提供 URL；可按需 `fetch`。源文本包含裸引用／Pure 表达式时，先使用 Runtime 的 `workflow.validate` 验证；通用 JSON Schema 不负责解析这些扩展表达式。

构建前安装 Rust 的 `wasm32-unknown-unknown` target 和 `wasm-bindgen-cli 0.2.128`，随后运行 `npm run build:wasm`。`WASM_BINDGEN` 可指定 CLI 路径。已有 WASM 构建后，`npm run build` 更新 SDK、类型、公开 Schema 和 SHA-256 manifest。浏览器集成测试运行 `npm run test:browser`，支持 `CHROME_PATH`；该测试使用仓库 UI 依赖中的 `ws`。
