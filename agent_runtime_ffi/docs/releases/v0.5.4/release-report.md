# v0.5.4 发布验证报告

日期：2026-11-09（Asia/Shanghai）。

执行源码提交：3e5ebf7；打包元数据提交：fb49a17。后续提交仅固定实际产物哈希与验证记录。

验证结果：

- Rust 工作区测试 1028 项通过、0 失败；36 项文档或外部环境测试沿用原有忽略设置。cargo fmt 检查通过。
- Lit 93 项、Svelte 55 项通过；类型检查与 Svelte 独立 JS 验证通过。
- Chrome Worker 15 项通过，涵盖 Agent、原生工具调用、审批、暂停继续、工作流与持久化。
- Windows DLL 实际加载并创建、关闭、销毁实例；Linux WSL 实际加载 SO，均返回 0.5.4、ABI major 1。
- Linux SO 联测暂停继续通过，3 次模拟模型请求，无合成用户消息。
- Rust SDK 11 项、Go SDK 编译、C++17 示例语法检查、Python SDK 语法检查通过。
- 最终 ZIP/TGZ、Svelte JS、Schema、头文件、WASM manifest、SDK 下载哈希均经逐文件核验。

模型和 RPC 使用本地模拟服务；浏览器验证环境为 Chrome。WASM 事件源订阅和会话快照恢复尚不支持。

| 附件 | 字节 | SHA-256 |
| --- | ---: | --- |
| orbit-runtime-runtime-v0.5.4-windows-x86_64.zip | 15772860 | 60b38e0a82852aee9b75b12e11cab077fcab25220c7ad725f788d3928d71cc06 |
| orbit-runtime-runtime-v0.5.4-linux-x86_64.zip | 15232428 | 29531d21f46a24661c739fa11e9c84e13d2818278d175e6a58399876108b2d43 |
| orbit-runtime-frontend-svelte-v0.5.4.zip | 2281171 | 330ba189ad6338068a962080f2400f506aa329e00352e2b0ef54e934be84b8af |
| orbit-runtime-browser-v0.5.4.zip | 2798746 | 66aff8bedf97157aacb2dc1efe61d839565fd141cffe0af16775810e8cdd451b |
| orbit-runtime-browser-v0.5.4.tgz | 2751521 | 192731d2c7edc2752020583a762921be72cfc2899c11598a7acbc286de5daf89 |
| orbit-runtime-sdk-v0.5.4.zip | 116542 | 5646cc87293f99d0b02b7bd9e28bb6ada9721a012c2e50610cee8f9899ba5661 |

Engineering Review

Architecture: 原生与 WASM 共用执行核心，宿主负责事件调度和资源归属。
Code: 汇总 beta 改动，维持现有 ABI。
Concurrency: 命令锁、恢复空闲检查、事务版本与会话授权保护有效。
Resource: 有界事件缓冲，按需读取与文件解码。
Audit: 恢复诊断、原始调用历史和附件校验可追踪。
Verification: 上述测试与包内容检查通过。
Risks: 外部宿主需同步前端和 SDK，移除恢复冲突后丢弃历史的兜底。
Decisions Needed: 无。
