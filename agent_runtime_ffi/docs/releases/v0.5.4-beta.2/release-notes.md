# Orbit Runtime 0.5.4-beta.2 本地测试包

包含 beta.1 的 RPC 事件源、动态新增 RPC 工具、RPC 元数据修复及 WASM 共享核心，加入暂停后继续和 Lit／Svelte 输入框修复。

## 本次更新

- Native DLL 与浏览器 WASM 新增 `conversation.resume`，传入 `{ "conversation_id": "..." }`。沿用现有上下文继续已明确暂停的焦点 Agent，返回 `decision`、`reason` 和 `command_id`。
- Runtime 快照新增 `can_resume`。普通空闲、运行中、正在暂停时拒绝继续，整理上下文期间返回 `compact_in_progress`。继续命令经过会话命令锁和 Agent 调度锁。
- Lit 与 Svelte 共用继续状态机，支持从 Runtime 快照恢复按钮，防止重复点击和过期快照重新启用按钮，保留服务端拒绝结果。
- Worker transport 直接接入继续命令。Tauri 宿主配置 `commands.resume`，HTTP 宿主配置 `resumePath`，将入口转发到 Runtime 的 `conversation.resume`。宿主需要更新 DLL／WASM 和前端 JS，并透传 `can_resume`。
- 去掉 Lit／Svelte 编辑区内部的矩形边框和焦点框，保留外层输入容器的焦点反馈。
- Grep／GrepJson 新增集群与会话两层读取授权。集群通过 `grep_root_ids` 限定根目录，宿主通过 `conversation.set_read_access` 原子替换会话子目录授权，通过 `conversation.revoke_read_access` 撤销。
- `grep.require_conversation_grants:true` 开启默认拒绝，缺少集群范围或会话授权时不能读取全局目录。兼容开关默认关闭。恢复会话需重新授权，正在执行的读取结束后授权更新才返回。配置见包内 `docs/zh/19-grep-read-access.md`。
- Svelte 的后续前端包已支持宿主扩展「＋」菜单、文件卡片及挂载时配置格式的文件拖放。宿主保存资源并提供读取工具，模型消息通过资源 ID 按需读取。

## 产物与接入

提供 Windows x86_64 Release DLL、匹配 SDK 源码、Svelte 独立 JS、Lit npm 包、浏览器 WASM ZIP／npm 包及 SHA-256。

SDK 在线自动下载仍指向正式 0.5.3。本地测试请显式指定本包 DLL 路径。Native ABI 继续为 1.3。WASM 事件订阅仍为 unsupported，事件处理和任务触发由宿主接入。

本包基于 `jiangrong/rpc-event-sources` 工作区构建，构建记录标记为 local-preview 和 worktree_dirty。打包不代表宿主已完成接入。
