# Orbit Runtime v0.5.3

合并动态追加 RPC 工具、RPC 元数据修复和浏览器 WASM SDK。
保留 0.5.2 的 Workflow JSON、Skill 事务和 Lit／Svelte 前端；Native ABI 继续为 1.3。

## RPC 工具

- Runtime 启动后可动态新增外部 gRPC endpoint。已有 endpoint ID 或工具名称禁止覆盖，新增失败保留当前目录。宿主保存接受的 endpoint 配置。
- 修复“工具目录可查，但 AI 请求构造报 active tool has no registered metadata”的问题。Conversation 与 Agent 继承所属 Runtime 的上下文，新增 RPC 工具的元数据对实际 AI 路径可见，同时保持实例隔离。
- 回归覆盖 BrowserOpenPage 和普通 RPC 工具，从模型请求、工具执行到最终回复的完整路径。

## 浏览器 WASM SDK

- 新增独立 `@orbit-runtime/browser` JS／TS SDK，专用 Web Worker 加载 Rust WASM，与 Native 共用 Agent／Workflow 核心。
- 提供宿主异步工具、流式模型请求、审批和取消，以及 IndexedDB 资源事务与 Web Locks 命名空间互斥。
- 通过 capabilities 明确功能边界。首版不支持会话恢复、Skill／集群热更新、图片导入、本机文件／进程／原生 gRPC；AI 不开放 Workflow 编辑，存在存活 Conversation 时拒绝 Workflow 写入。
- 发布 ZIP 和可 `npm install ./文件.tgz` 的本地 npm 包，没有发布到 npm 公共仓库。内含类型、Schema、完整示例、中文规范和逐文件 SHA-256 manifest。WASM 调试名称段已移除。

## 附件

- `orbit-runtime-runtime-v0.5.3-windows-x86_64.zip`
- `orbit-runtime-runtime-v0.5.3-linux-x86_64.zip`
- `orbit-runtime-frontend-svelte-v0.5.3.js` 和 `.zip`
- `orbit-runtime-browser-v0.5.3.tgz` 和 `.zip`
- `orbit-runtime-sdk-v0.5.3.zip`

各文件附 SHA-256。WASM 使用说明见浏览器包的 `USAGE.zh-CN.md`，也单独附在 Release。
Svelte 是界面产物，浏览器 Runtime 仍需单独加载 WASM SDK。Native 宿主继续使用 DLL／SO。
