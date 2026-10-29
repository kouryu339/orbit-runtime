# Orbit Runtime 0.5.3 WASM 使用规范

JS／TS 宿主通过 `@orbit-runtime/browser` 在专用 Web Worker 中运行 Runtime。
Agent 状态机、工具元数据、权限和 Workflow 编译执行共用 Rust 核心。
浏览器使用 Promise 和事件订阅；Native SDK 继续加载 DLL／SO，C ABI 仍为 1.3。
Svelte JS 是对话界面，WASM 包是执行 Runtime，二者需要分别接入。

## 获取与安装

从 GitHub 的 `v0.5.3` Release 下载：

- `orbit-runtime-browser-v0.5.3.tgz`：项目内安装。
- `orbit-runtime-browser-v0.5.3.zip`：同一组文件，用于静态托管。

安装下载的文件：

```sh
npm install ./orbit-runtime-browser-v0.5.3.tgz
```

没有发布到 npm 公共仓库，不能假定 `npm install @orbit-runtime/browser` 能取得本版。
ZIP 解压后根目录是包目录，保留以下相对关系：

```text
package.json
README.md
USAGE.zh-CN.md
examples/quickstart.js
dist/
  index.js                 # SDK 入口
  index.d.ts               # TypeScript 类型
  worker.js
  storage.js
  manifest.json            # 版本、文件大小、SHA-256
  schemas/*.schema.json
  wasm/
    orbit_runtime_wasm.js   # wasm-bindgen 模块
    orbit_runtime_wasm_bg.wasm
    *.d.ts
```

不要只拷贝 `.wasm`，也不要把 Worker 或绑定 JS 混用其他版本。
Release 的 `.sha256` 检查下载文件，`dist/manifest.json` 检查解压后的文件。

## 最小接入

`examples/quickstart.js` 是完整的模型、Role、集群和宿主工具配置示例。
静态托管时可以这样调用；项目内也可参考它直接导入 `@orbit-runtime/browser`：

```js
import { openExample } from '/orbit/examples/quickstart.js';

const session = await openExample({
  baseUrl: location.origin + '/model-proxy/v1',
  apiKey: userSessionToken,
  modelId: yourModelId,
  onEvent(event) { /* 将事件交给宿主或 Conversation transport */ },
});

await session.send('请用 WebEcho 返回 hello');
// 用户操作时：await session.pause();
// 离开项目时：await session.close();
```

`send` 返回表示命令已接受，不等于模型和工具已全部结束。
通过事件或 `conversation.materialize` 判断最终状态。
生产宿主按状态禁用重复发送、批准、暂停和关闭操作，并处理 Promise 拒绝。

自己组装配置时：

1. `createRuntime` 创建 Worker，加载资源和模型配置，注册集群及宿主工具，启动后返回。
2. `runtime.subscribe` 订阅事件，保留返回的取消函数。
3. `conversation.spawn` 使用 `cluster_id` 创建会话；主 Agent 必须绑定已注册的 Role Skill。
4. 通过 `runtime.invoke(type, payload)` 发送消息、批准、暂停、关闭。
5. 离开时先关闭会话，再取消订阅并 `await runtime.dispose()`，释放 Worker、资源锁和存储连接。

## 配置与输入

| 字段 | 内容与约束 |
| --- | --- |
| `namespace` | 非空、最长 128 字符；作为当前源站下的资源分区，建议包含账号和项目稳定标识。 |
| `skills` | SKILL.md 正文数组；不接受本机路径。Role／Feature 名称唯一，不覆盖系统 Skill。 |
| `workflows` | 完整 `orbit.workflow.resource/v1` 源文本数组；允许已定义的裸引用和 Pure 语法。 |
| `llm` | `agent-runtime-llm-registration/v1`；启动时重新提供，API Key 不写入资源快照。 |
| `clusters` | `agent-runtime-agent-cluster-registration/v1` 数组；每个 Agent 绑定 Role Skill。 |
| `endpoints` | `{id, protocol:'host', timeout_ms?}` 数组；超时范围 1～60000 毫秒。 |
| `host` | 异步处理 `rpc.list_tools` 和 `rpc.execute`，处理取消信号。 |
| `storage` | 默认 IndexedDB；`'memory'` 为明确的临时模式，关闭后不保留资源。 |

集群首版接受 `id/description/focus_agent_id/agents/max_thinking_rounds/permissions`；
Agent 接受 `id/name/role/features/model_uid/system_skills/tool_protocol`。
未知字段会报错；不要直接把包含文件路径、进程配置的 Native 注册配置传进来。
Skill、集群和模型注册在启动前完成，首版不支持运行中的内容更新。

`runtime.capabilities.commands` 和 `unsupported` 是判断功能是否开放的依据；
不要仅凭版本号推断 Native 与浏览器命令完全相同。

## 宿主工具

浏览器不直接连接原生 gRPC；使用 `host` 回调接浏览器能力、后端或 WebSocket 桥。
动态新增使用 `runtime.add_rpc_endpoint`，`endpoint.protocol` 必须是 `host`。
已有 endpoint ID 和工具名称不能覆盖或修改，新目录注册失败时保留旧目录。
动态增加的 endpoint 不包含在资源快照，重新创建 Runtime 时需重新提供。

工具发现返回：

```js
{
  schema: 'corework-agent-tool/v1',
  tools: [{
    name: 'WebEcho', description: '返回文字。', readonly: true, idempotent: true,
    parameters: [{ name: 'value', param_type: 'String', required: true }],
    outputs: [{ name: 'value', field_type: 'String' }],
  }],
}
```

执行回调接收 `payload.endpoint_id`、`payload.metadata` 和 `payload.request`。
请求字段是 `tool_name`、`args_json`、`args_cli`、`call_id`、`tool_call_id`、
`idempotency_key`，以及当前会话／Agent／工作流等身份字段；以实际请求为准。
不要把模型提供的参数当作已授权的任意 URL、文件路径或后端指令执行。
宿主仍负责身份、权限和业务校验。

成功结果为 `{result: {...}, to_ai: '简短说明', error_code: 0}`；`to_ai` 必须非空。
业务失败返回非零 `error_code`，传输失败可抛出错误。
将 `signal` 传给 `fetch` 等可取消操作；对外部副作用使用幂等键，
因为浏览器取消等待不保证远端已经发生的动作撤销。

## 持久化与并发

默认持久化 Workflow 源、Skill 正文和集群配置，使用 IndexedDB 事务和 revision 检查。
存储失败会使初始化或写操作失败；不自动切换临时内存模式。
持久化资源不是会话历史快照，首版不能刷新页面后恢复原会话执行。

同一源站下，相同持久化 `namespace` 只允许一个存活 Runtime，由 Web Locks 互斥。
不同项目使用不同 namespace；旧实例释放前不要启动同 namespace 的第二个实例。
自定义持久化适配器也需要 Web Locks，并遵守 `ResourceStore.save(snapshot, expectedRevision)`
约定：比较版本、整体原子提交、成功后返回新 revision，失败不得部分保存。
显式 `storage:'memory'` 不使用持久化 namespace 锁。

重建时已保存的 Workflow 优先于 `options.workflows`；`skills` 和 `clusters` 如显式提供，
则优先于保存内容。模型凭据、endpoint 和 `host` 回调始终由宿主重新提供。
初始化成功也会提交一次资源快照。清理浏览器数据、隐私模式或存储配额可能影响保留期；
重要资源由宿主另做导出或服务端保存。

Workflow 对外提供 `list/register/remove/validate/promote/execute`。
源文本通过 `workflow.validate` 校验；普通 JSON Schema 不解析裸引用／Pure 扩展。
首次浏览器版本有存活 Conversation 就拒绝 Workflow 写入，包括等待和暂停的会话；
先关闭相关会话再登记、移除或转正。AI 侧不开放 Workflow 写入工具。
Workflow 写入先校验，再等待存储确认；失败回滚目录并丢弃候选成功事件。
存在执行中的 Workflow 时同样受执行占用保护。

## 部署

- 使用支持 WASM、模块 Worker 的现代浏览器，在 HTTPS 或 localhost 下运行；不要用 `file://`。
- 默认持久化额外需要 IndexedDB 和 Web Locks。当前端到端验证使用 Chrome，其他浏览器需在宿主产品中验证。
- Worker、绑定 JS 和 WASM 保持相对路径；JS 作为模块提供，WASM MIME 为 `application/wasm`。
- Vite 等打包器需要输出独立模块 Worker 和 `.wasm` 资源；不要只复制 SDK 入口。
  默认入口使用静态可识别的 `new Worker(new URL(..., import.meta.url), {type:'module'})`。
- 可通过 `workerUrl` 指定 Worker，通过 `wasmModuleUrl` 指定 **绑定 JS 模块地址**；
  后者不是 `.wasm` 文件地址。绑定模块会按自身位置加载对应 `.wasm`。
- 模型请求在 Worker 中发出，遵守浏览器 CORS 和页面 CSP。宿主代理须允许实际源站、
  请求头及所用 SSE 协议；CSP 需允许 Worker、模型连接和 WASM 编译。
- 服务器可开启 gzip／Brotli，并为带版本的静态文件设置缓存。压缩下载量、解压后
  WASM 大小和执行内存是不同指标；实际文件大小见 manifest。
- 不向浏览器部署服务端长期密钥；示例中的 token 应由宿主获取。
  “API Key 只驻留内存”不意味着页面代码、扩展或调试工具无法读取。

## 本版能力边界

| 已接入 | 尚未接入 |
| --- | --- |
| Agent／多成员集群、流式模型请求、原生工具调用 | 会话历史重载恢复 |
| 宿主工具发现、追加 endpoint、工具审批和取消 | 原生 gRPC、本机文件、进程和环境变量能力 |
| Workflow 编译执行、变量、FOR、Switch、Pure | Skill／集群内容热更新 |
| IndexedDB 资源重载、版本互斥、写入回滚 | 图片文件导入 |
| Lit／Svelte 共享 Worker transport | AI Workflow 编辑、存活会话期间 Workflow 写入 |

Lit／Svelte 的源码包可使用 `WorkerConversationTransport`；独立 Svelte JS
仍由宿主传入符合其协议的 transport。组件展示隐藏原始工具 JSON 不代替数据访问控制。

## 构建与检查

源码构建需要 Rust `wasm32-unknown-unknown` target、Node.js 20+、
`wasm-bindgen-cli 0.2.128` 和仓库 Cargo.lock。仓库当前不跟踪 Cargo.lock，
发布验证使用构建时固定的锁文件；复现依赖需保留该文件。

```sh
cd sdk/runtime/js
npm run build:wasm
npm test
npm run test:browser
```

`WASM_BINDGEN` 可指定 CLI 路径，`CARGO_TARGET_DIR` 可指定构建缓存。
构建脚本使用 `--locked`，缺少或改变锁文件时，先在仓库根运行 `cargo metadata --format-version 1`
生成／确认锁文件，再重试。发布输出移除名称和 producers 调试段，不改变执行能力。
`npm run build` 仅在已有 WASM 时刷新 JS／类型／Schema／manifest。
浏览器测试需安装仓库 UI 依赖中的 `ws`，可用 `CHROME_PATH` 指定 Chrome。

排查时记录 `runtime.version`、`runtime.capabilities`、namespace、错误 kind／message、
`command_id` 和会话 ID。不要把 API Key 或完整业务工具参数写入普通用户日志。
