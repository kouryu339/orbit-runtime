# Orbit Runtime v0.5.4

汇总 0.5.4-beta.1、beta.2 和后续本地修复，正式提供 Windows x86_64 DLL、Linux x86_64 SO、浏览器 Worker WASM、Svelte 独立 JS 与匹配 SDK。

## Runtime 与 RPC

- 原生 Runtime 增加事件源目录、订阅、轮询、确认和取消。采用有界缓冲与显式 ACK；宿主负责持久化、去重及任务触发。
- 保留动态新增 RPC 端点及工具目录能力；不支持覆盖既有端点。AI 请求使用注册表内的 RPC 工具元数据。
- 新增 conversation.resume 和快照 can_resume。已明确暂停的会话可继续；运行中、暂停中及上下文整理中拒绝不适用的操作。
- 会话恢复默认使用当前集群最新版 Role、thinking 和 Feature/Capability Skill，不因 Skill 正文哈希变化阻止恢复。保留用户消息、AI 原文、工具调用、结果、调用 ID 和顺序。
- 恢复时重建提示词和工具权限，移除失效的可选能力并返回 recovery 诊断；必需 Skill 缺失明确报错。未完成调用重新校验，已完成调用及结果未知的副作用调用不自动重放。Skill 保存的并发版本检查保持有效。

## 文件读取与宿主扩展

- Grep/GrepJson 支持集群 grep_root_ids 与宿主会话授权的交集。conversation.set_read_access 原子替换授权，conversation.revoke_read_access 撤销授权。
- grep.require_conversation_grants 可启用默认拒绝；兼容模式默认关闭。会话恢复后宿主需要重新授予读取权限。
- 增加原生宿主文件解码注册接口和 Rust SDK 支持，输出文本或 JSON 检索表示；保留内置格式处理。解码服务仅在本机回环通道工作，须在启动前注册。

## Lit、Svelte 与 WASM

- Lit/Svelte 共用暂停、继续和快照恢复逻辑；HTTP、Tauri 和 Worker transport 提供对应命令接入。
- 移除输入区内部边框。Svelte 支持宿主扩展「＋」菜单、文件卡片，以及按挂载时允许格式接收文件拖放。
- Svelte 构建直接引用共享状态源码，避免产物携带旧版 can_resume 逻辑。
- 浏览器 WASM 包包含 JS/TS SDK、Worker、WASM 及 Schema。事件源订阅仍为原生能力，浏览器端明确返回 unsupported。

## 升级接入

同步更新 Runtime、前端 JS 和 SDK，并透传 can_resume 与 resume 命令。删除宿主“Skill 冲突就新建空会话”的兜底，恢复原始历史并展示 recovery 诊断。Native ABI 保持 1.3（ABI major 1）。

Windows 和 Linux 包中的动态库需解压使用；浏览器请使用完整 WASM 包，不单独复制 .wasm。Svelte 的 .js 附件可直接用于宿主集成。
