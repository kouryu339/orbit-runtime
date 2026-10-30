# Orbit Runtime 0.5.4-beta.1 本地测试包

包含 0.5.3 已合并的浏览器 WASM SDK／共享核心、RPC Agent 元数据修复，以及本轮 RPC 事件源扩展。
本次只构建 Windows x86_64 DLL 和匹配 SDK 源码，不生成新的浏览器、Svelte 或 Linux 产物，不上传远程发布。
Native ABI 继续为 1.3，SDK 在线下载默认仍指向正式 0.5.3，测试请显式指定本包 DLL 路径。

## 事件源

- 独立 EventSourceService，提供 ListEventTypes、Subscribe 和 Acknowledge；支持纯事件插件，也可与工具共用地址。
- Native 新增 runtime.add_event_source、runtime.get_event_source_definitions、runtime.subscribe_events、runtime.poll_source_events、runtime.acknowledge_events、runtime.unsubscribe_events。
- 有界接收缓冲，显式确认后才释放容量。未确认事件可重复读取；恢复必须绑定原 stream_id，游标过期或日志重置明确报错。
- 支持可重放来源的断线重连、实时来源的断线缺口状态、取消和关闭。事件不会自动注册为 AI 工具，也不会自动启动任务。
- Node 插件 SDK 和 Python／Rust／Go／C++ 宿主 SDK 提供相应接入；公开 proto、请求 Schema 与中文规范随包分发。

宿主负责持久化 inbox／checkpoint、业务过滤、任务绑定和去重；可靠保存后再 ACK。
微信 Hook 仍需由微信宿主接入事件源。本轮没有发送真实微信消息或连接真实模型。
浏览器 WASM 接入源码包含在合并基线中，WASM 事件订阅仍明确为 unsupported。

将匹配 SDK 包解压后使用其中 Python 等宿主 SDK；事件插件开发可使用附带的 Node SDK 源码包。
详细契约见 Native 包 docs/zh/18-rpc-event-sources.md。
