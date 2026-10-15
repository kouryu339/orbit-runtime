# v0.5.1 相对 v0.5.0 的版本更新报告

- 对比基线：[v0.5.0](https://github.com/kouryu339/orbit-runtime/releases/tag/v0.5.0)。
- 本次版本：[v0.5.1](https://github.com/kouryu339/orbit-runtime/releases/tag/v0.5.1)。
- [版本差异](https://github.com/kouryu339/orbit-runtime/compare/v0.5.0...v0.5.1)。
- 发布日期：2026-10-15（Asia/Shanghai）；原生 ABI 继续为 1.3。

## 修复与行为变化

原生 FC 和旧 EXEC 路径在模型响应已经返回后，仍会检查一个默认 20 步的
`AUTO_CONTINUE_MAX_STEPS` 限制。连续接纳 20 个工具批次后，下一次模型响应若仍含工具
调用，就被改写为等待用户继续；该次模型请求已经消耗资源。

本版删除这条后置限制及其状态和提示，保留 thinking 入口的 `MAX_THINKING_ROUNDS`。
默认值仍为 0，表示不启用轮数限制；配置正数时，超限在本轮模型请求之前产生兜底回复。
历史快照里的旧步数键不参与新版本执行。工具审批、执行准入、token 上下文预算和暂停路径不变。

宿主如需有限轮数，应显式配置正数 `max_thinking_rounds`；默认 0 不再有隐含的 20 步兜底。

## 验收

- `cargo fmt --all -- --check`、Rust 工作区串行测试通过；AI Assistant 349 项、
  Runtime 137 项及其余工作区、集成和文档测试通过。需要真实 API 凭据的既有测试未运行。
- 新回归覆盖 FC / EXEC 连续 25 个工具批次，以及含旧计数键的缓存；正数轮数耗尽
  在模型配置或请求之前返回。
- Go Runtime SDK（CGO_ENABLED=0）检查、C++17 示例编译、Python SDK 语法及版本/下载元数据检查通过。
- Windows DLL 加载、版本 0.5.1、ABI 1.3、注册、启动、会话创建、关闭及销毁通过。
- Windows DLL 配合本地模拟模型完成真实 FFI/SDK/HTTP/工具/ledger 往返：
  默认轮数 0 执行 25 次成功工具调用，以第 26 次模型请求生成最终回复；轮数设为 2
  时仅发送 2 次模型请求、产生 2 个成功工具结果并停驻，没有第 3 次请求。
  该验证未使用真实模型凭据或业务写入工具。
- Linux x86_64 交叉编译、链接、ELF 和 ZIP 检查通过；未执行 Linux 动态加载。
- 发布构建使用隔离源码快照：基于 main 的 `a9b235e` 和本次修复，排除工作区未提交 UI 产物。
  两个包均核对精确文件清单、头文件及原生库字节、SHA-256 附件和各语言 SDK 元数据。

| 平台 | ZIP 字节数 | SHA-256 |
|---|---:|---|
| Windows x86_64 | 14187669 | f8fefd3972e192866ac4368247810a18940885564df26d35bfb79720fa886d34 |
| Linux x86_64 | 13982284 | 8d16826170367a405d01a31ec8e39df54068b32061e62a790648a24e7e5bc4ec |

每个平台另附 `.zip.sha256` 文件。远端正式标签和附件以 GitHub Release 为准。

## Engineering Review

Architecture: 轮数预算保留在 thinking 请求前，移除响应后的独立步数门槛。
Code: 删除旧计数器、重置、停止分支和中英文提示，不新增预算抽象。
Concurrency: 保持现有每 Agent 状态与工具调度边界。
Resource: 避免后置限制丢弃已消耗资源生成的工具调用；默认 0 不限制轮数。
Audit: 保留现有轮数日志、LLM usage 和工具调用 ledger。
Verification: 新增双协议连续 25 批及旧快照计数回归、正数轮数前置停止回归。

Risks: 宿主需要有限执行时必须配置正数轮数预算；Linux 动态加载未实测。
Decisions Needed: 无，保留默认 0 已由使用方确认。
