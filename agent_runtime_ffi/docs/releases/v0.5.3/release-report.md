# v0.5.3 发布验证报告

发布日期：2026-10-29（Asia/Shanghai）。
范围：Windows x86_64、Linux x86_64、Svelte 独立 JS、浏览器 WASM SDK 和匹配源码 SDK。

执行源码提交 `dfb77f22ff971544e06d2ed94644008528a62a6b`，分支 `jiangrong/release-0.5.3`，合并动态追加 RPC、RPC Agent 元数据修复与浏览器 WASM 接入。
原 WASM 工作区与分支保留。此发布没有修改 main；正式标签位于发布分支。
Native build-info、前端 manifest 与 WASM manifest 对应同一执行源码提交。
最终标签在其上只补 SDK 下载版本／实际 ZIP 哈希和本报告，未改变已构建的执行代码。

## 验证结果

- Rust 格式检查通过；工作区库与集成测试 983 通过、0 失败，1 项需要真实模型凭据的测试按原规则忽略。
- 下载元数据更新后另复核 Rust SDK 11 项通过，未重复计入上述总数。Go SDK 在 CGO_ENABLED=0 下编译通过；C++17 示例和 Python SDK 语法检查通过。
- Lit 87、Svelte 51 项测试通过；两套类型检查、生成样式一致性与 Svelte 独立 JS 挂载／发送／销毁验证通过。
- Windows DLL 和 Linux SO 重新构建；最终 ZIP 解压后的两种原生库均实际加载，确认 Runtime 0.5.3、ABI 主版本 1。
- Windows 最终 DLL 的 BrowserOpenPage 和普通 PlainRpcProbe 各发送 2 个模拟模型请求、执行 RPC 1 次并产生最终回复。工具定义、会话／Agent／调用身份通过检查。
- Windows 动态 RPC 新增、重复 endpoint／工具名拒绝、目录事件、Workflow 编译执行及宿主配置重启恢复通过。
- Windows 最终包的 Workflow 登记／验证／转正／执行／移除、裸引用／Pure、FOR／Switch、重启恢复、旧接口拒绝、无效资源保留通过。
- Windows 最终包的动态集群、存活会话占用保护、Skill 重读与恢复版本冲突通过；Skill 候选事务与 Workflow 引用链执行占用保护通过。
- Linux WSL 实际加载 SO，完成 Skill 事务、新进程资源恢复和 Workflow 占用保护；最终 ZIP 中的 SO 再次独立加载成功。
- 真实 Chrome Worker 完成 14 项检查：工作流编译执行、共享控制流、动态宿主工具、双成员 Agent、SSE／原生工具调用、审批、暂停关闭取消、命名空间互斥、IndexedDB 重载、失败回滚和配套示例。
- WASM SDK 内存存储 revision 单元检查、TypeScript 接入检查通过。Vite 源码接入真实浏览器通过；最终 TGZ 离线安装后重新构建消费者项目，真实浏览器初始化、版本和 Workflow 执行通过。
- `scripts/verify-release-artifacts.py` 核对 ZIP 完整性、逐文件与源码一致、原生格式、头文件／Schema／语法、前端独立 JS、浏览器 ZIP 与 TGZ 文件等价、WASM manifest 哈希和 SDK 下载固定哈希，全部通过。

本轮模型使用本地 HTTP 模拟服务，工具使用本地 gRPC／浏览器宿主回调，没有连接真实模型或业务系统。
浏览器端到端验证环境为 Chrome；不宣称已验证其他浏览器。Linux RPC AI 网络链路未单独端到端执行，已验证原生加载、资源事务与共享 Rust 回归。

## WASM 体积与接入

最终 WASM 9,081,044 字节，gzip level 9 为 2,685,836 字节；调试名称和 producers 段已移除。
TGZ 包约 2.75 MB。
压缩下载量、解压后的模块大小、运行内存分别计量，不能互相替代。

规范见 `sdk/runtime/js/USAGE.zh-CN.md`，随浏览器包与 Native 包分发，另附独立文档。
首版不支持会话恢复、Skill／集群热更新、图片路径导入、本机文件／进程／原生 gRPC。
有存活 Conversation 时拒绝 Workflow 写入；AI 不开放 Workflow 编辑。
默认 IndexedDB 与 Web Locks，持久化失败不会静默退化；同 namespace 只允许一个存活 Runtime。

## 附件

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| orbit-runtime-runtime-v0.5.3-windows-x86_64.zip | 15404467 | d72c66bfffdb7766b360584f619ae2e4a665dec89163c08f791511d6d1c93b3e |
| orbit-runtime-runtime-v0.5.3-linux-x86_64.zip | 14934303 | d12bcda3192405453fb0d2c214a4b150d8ad847c57941a696875506fd6a6cea4 |
| orbit-runtime-frontend-svelte-v0.5.3.zip | 2276043 | c7cffa26895d17ca411a88e39d04c708bdb48157104b5ef14e04fba5a22dcb49 |
| orbit-runtime-browser-v0.5.3.zip | 2799171 | f9a53fcba4bebd1c50295e8c1282f4d1d3af2bce63ee16a5049dc8c95d44254d |
| orbit-runtime-browser-v0.5.3.tgz | 2753260 | d21b4574ef73a4d4ae329f08fe0466f13bf24a64e0e9534fa7f285f9efb3098b |
| orbit-runtime-sdk-v0.5.3.zip | 111753 | 76202cfc400ef30e5d848a540061f8f6a5f0999b0687fb819ed332ea7553620b |
| orbit-runtime-frontend-svelte-v0.5.3.js | 6986951 | 59b09d64f3477f582f5742d2f66e4653edf27059e010fb4b098db75d74f9838c |

各附件另附 `.sha256`。`verification.json` 保存本地检查摘要；远程发布后另核对标签目标、Latest／prerelease 标记及所有上传附件字节。
源码 SDK 与原生包分开，避免下载哈希的循环依赖。

## Engineering Review

Architecture: Native 与浏览器仅入口和宿主适配不同，共用 Agent／Workflow 核心；Conversation 继承所属 Runtime 的完整 FrameworkState。

Code: 保留现有 C ABI，增加独立 Promise／Worker SDK、内容存储边界与匹配打包校验，未复制执行器。

Concurrency: 工具目录保持实例隔离、追加注册；浏览器写入串行、revision 事务和 namespace 独占，等待外部工具时审批／暂停／关闭仍可处理。

Resource: 复用构建缓存，SDK 不依赖 UI 框架；移除 WASM 调试名称段，分发压缩资源并支持按版本缓存。

Audit: 保留源码提交、build-info、WASM 逐文件 manifest、固定下载哈希与实际 DLL／SO／Worker 验证摘要。

Verification: 983 项 Rust、138 项前端、14 项 Chrome Worker、TGZ 安装消费者、实际 Windows／Linux 加载与事务检查通过。

Risks: 浏览器首版能力有限；取消不保证远端业务副作用撤销，宿主需实现授权及幂等。没有执行真实模型／业务工具测试。

Decisions Needed: 无，按已授权的 0.5.3、Windows＋Linux＋Svelte＋WASM 范围发布；保留原分支，未改 main。
