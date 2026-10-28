# v0.5.2 发布验证报告

发布范围：Windows x86_64、Linux x86_64、Svelte 独立 JS 与源码 SDK。
变更包括 Workflow JSON、动态集群、Skill 候选事务，以及 Lit/Svelte 对话前端。
发布日期：2026-10-28（Asia/Shanghai）。

构建对应源码 `899b7e20824f784a12653261c13481c5b1ee5932`，来自
`jiangrong/lit-svelte-frontend`；已合并 `jiangrong/workflow-logic-json` 的
`83bd97a`，原 Workflow 分支保留。打包时工作区干净，原生包的
`build-info.json` 和前端包的 `manifest.json` 记录此提交。
最终发布标签在该提交之上只补 SDK 下载版本／哈希与本报告，不改变已构建的执行代码。

## 验证

- Rust 格式检查通过。FFI 152、AI Assistant 355、Corework 379 项单元测试通过；
  49 项集成测试和 11 项 Rust Runtime SDK 测试通过，共 946 项。
- Lit 85、Svelte 51 项前端测试通过；两套类型检查和生成样式一致性检查通过。
- Windows DLL 和 Linux x86_64 交叉编译成功；实际加载打包的 Windows DLL，
  确认版本 0.5.2、ABI 主版本 1，以及 Skill 事务、正式编辑转草稿、执行占用保护能力。
- 使用解压包与匹配的 Python SDK，验证 Workflow 登记／验证／转正／执行／移除、
  重启恢复、裸表达式／Pure、循环／switch、旧接口拒绝、无效资源启动失败与原文件保留。
- 验证动态集群增改、存活会话占用、关闭后更新、Skill 重读、历史快照版本冲突。
  候选事务验证包含新增 Role／Feature 和集群、跨集群共享 Skill 占用、失败保留、
  陈旧版本拒绝、进程重启恢复及工作流占用保护。
- Svelte 发布文件实际经 ES Module 导入，完成组件挂载、发送、断开和重复销毁检查；
  确认字体内置，文件集合没有额外 JS 分块，独立 JS 与 ZIP 内入口字节完全一致。
- Go SDK 在 CGO_ENABLED=0 下通过编译检查；C++17 示例语法检查和 Python SDK 编译检查通过。
- `scripts/verify-release-artifacts.py` 验证原生文件和头文件、Schema、语法文档与源码
  字节一致，Linux ELF 为 x86_64，SDK 下载版本／SHA-256 与实际附件一致；
  ZIP 完整性、文件清单、前端 manifest 中逐文件哈希和相邻 SHA-256 文件通过。

本轮没有调用真实 LLM 或业务工具；Linux 只验证交叉编译和 ELF／包内容，未实机动态加载。

## 附件

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| orbit-runtime-runtime-v0.5.2-windows-x86_64.zip | 15264498 | b86a1bff4f79d175be13d137c006cd698c92e8b56249e71c399733620ee06228 |
| orbit-runtime-runtime-v0.5.2-linux-x86_64.zip | 14777247 | 55d7e91855c3f9d7f108e06429cbcb2d1c50ba2b27085ac308f013811fb8fb88 |
| orbit-runtime-frontend-svelte-v0.5.2.zip | 2275556 | c84cd499b57b18c1d2485df5191cff43a2498987ff9b8982ea3acfc971b54a1a |
| orbit-runtime-frontend-svelte-v0.5.2.js | 6984607 | 32e4739189f8a348424bc105cec17bff53937d631bb864cba72104f55184d304 |
| orbit-runtime-sdk-v0.5.2.zip | 83093 | 660fe08522df75e446177138d64728d078c3941daac81ef4a3c8eb70b172fee3 |

每个文件另附 `.sha256`。SDK 独立成包，保证原生 ZIP 哈希可固定后再生成 SDK，
消除原生包内嵌自身下载校验值的循环依赖。对应版本的源代码与类型定义由发布标签提供。

## Engineering Review

Architecture: Workflow 逻辑资源由 Runtime 编译和持久化，沿用原图执行核心；两套前端共享协议，独立渲染。SDK 下载元数据与原生包分离。

Code: 合并已完成分支，增加单文件 Svelte 构建入口和通用打包／附件验证；未新增业务执行层。

Concurrency: 保留 revision、集群和 Skill 占用检查、工作流引用链执行保护；UI 的发送／暂停／继续和弹窗请求有重复操作防护。

Resource: 前端依赖、样式和字体打入单文件，避免外部 CDN；压缩包约 2.3 MB，宿主可缓存或压缩传输独立 JS。

Audit: 原生 build-info、前端 manifest、源码提交、SDK 固定哈希及发布附件 SHA-256 可对照。

Verification: 946 项 Rust、136 项前端测试，真实 Windows FFI 闭环、编译产物挂载和附件完整性检查通过。

Risks: 旧 Workflow 脚本／图资源需重新登记；两个 Studio 页面已弃用；Linux 未实机加载。前端隐藏原文不替代宿主的数据访问控制。

Decisions Needed: 无，v0.5.2 与 Windows＋Linux＋Svelte 范围已确认。
