# 浏览器 WASM SDK 实施

目标：JS／TS 使用独立浏览器 SDK；Native SDK 继续加载 DLL／SO。命令、事件与 Workflow 源语法共用，Rust 业务逻辑只保留一套。浏览器计算在专用 Worker 中执行。

实现分支：`jiangrong/browser-wasm-sdk`。基线 `f8de634` 保存动态 RPC beta；随后与 RPC 元数据修复汇总到 `jiangrong/release-0.5.3`，作为 0.5.3 发布。

实施顺序：

1. 为共享核心拆分 Native 与浏览器依赖、时钟、任务与取消；WASM 实际编译验证。
2. 为资源提供内容存储边界；浏览器以事务形式保存，禁止用系统文件路径冒充浏览器资源。
3. 新增 WASM 入口与 JS／TS Worker SDK，沿用命令和事件，明确 capabilities。
4. 接入宿主异步工具、网络流、权限、会话和恢复；Lit／Svelte 共用传输适配。
5. 同一批 Workflow/Agent 用例验证 Native 与浏览器结果；生成独立 WASM 包、类型、Schema、manifest 和哈希。

并发约束：网络等待不持有命令入口全局锁；暂停、批准、关闭可以继续处理。异步宿主请求有唯一调用 ID、期限、取消和响应次数检查。一个 Worker 负责一个 Runtime；相同存储命名空间的写入必须有独占所有权。

Native 同步 ABI 继续由 Native 入口驱动异步核心。浏览器 SDK 使用 Promise，不在 Worker 中阻塞等待宿主。浏览器中的磁盘、进程、原生 gRPC 能力按 capabilities 明确报告，不能静默假装支持。

首版实现已经完成：`agent_runtime_wasm` 是独立绑定入口，`sdk/runtime/js` 是浏览器 SDK，Lit 与 Svelte 共用 Worker transport。Workflow 源存储抽象保留 Native 文件实现，浏览器使用内容存储和事务确认。模型注册及节点元数据投影已提取为共享逻辑。

真实 Chrome Worker 已验证：工作流编译／执行、FOR／Switch／Pure／变量、动态宿主工具、双 Agent 集群、SSE 和原生工具调用、审批、暂停／关闭取消、IndexedDB 重载、命名空间互斥、失败回滚。Native 的库和集成回归也已执行；最终报告随本地产物附带。

异步持久化仍有明确边界：首版浏览器不开放 AI 工作流写入工具，存在存活 Conversation 时拒绝工作流写入，避免 Agent 绕过存储确认使用候选图。会话恢复、Skill／集群热更新和图片导入尚未接入；不宣称与 Native 所有命令等价。这些限制在 capabilities 中公开。浏览器入口不加载本机文件、不启动本机进程，不静默兼容 Native 路径参数。

接入及构建方式见 `sdk/runtime/js/README.md`；使用规范与注意事项见 `sdk/runtime/js/USAGE.zh-CN.md`。
