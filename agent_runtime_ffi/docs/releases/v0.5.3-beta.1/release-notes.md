# Orbit Runtime 0.5.3-beta.1 本地测试版

本包用于本地试用，不创建远程发布或标签。原生 ABI 保持 1.3。

- 新增宿主管理接口 `runtime.add_rpc_endpoint`，启动后连接新的外部 gRPC 服务，通过 ListTools 整体登记工具集。
- 已存在端点 ID、工具名、内置工具和 WorkflowRef 名称冲突时，拒绝整个请求；不提供更新、刷新或移除。
- 发现失败或超时不改变旧目录；发现期间释放操作锁，提交时重验并发冲突与关闭状态。
- 新工具可用于现有上下文和 Workflow 编译、执行；活动工具列表和审批要求继续生效，新增不自动授权。
- Python、Rust、Go、C++ SDK 增加调用方法和目录变更事件。原生包包含请求 Schema、中英文接口说明。

## 试用

将 Windows 包中的 `bin/agent_runtime.dll` 替换到测试宿主的 DLL 路径，或通过 SDK 显式指定其绝对路径。
SDK 默认在线下载仍固定正式版 0.5.2；此测试包不会自动从网络下载。

```json
{
  "schema": "agent-runtime-command/v1",
  "type": "runtime.add_rpc_endpoint",
  "payload": {
    "schema": "agent-runtime-rpc-endpoint-add/v1",
    "endpoint": {
      "id": "project-tools",
      "protocol": "grpc",
      "endpoint": "http://127.0.0.1:55061",
      "timeout_ms": 30000
    }
  }
}
```

服务必须先启动并提供标准 AgentToolService。成功后可通过既有端点、工具和 Workflow 节点目录查询。
宿主保存成功的端点配置，重启时在 Runtime.start 前登记，以恢复依赖这些工具的 Workflow。
完整参数见 `docs/zh/17-dynamic-rpc-additions.md`。

Svelte 独立入口为 `orbit-conversation-svelte.js`，同时提供 ZIP 和可直接导入的 JS 文件。
本次 RPC 改动不增加前端操作页面。

## 已知范围

动态新增只开放外部 gRPC；托管进程、动态检索绑定和连接池尚未纳入。
原有全局 Workflow 所有权仍限制在同一进程同时启动多个完整 Runtime。
Runtime 拒绝修改已登记契约，但无法约束远端服务自行更改实现。
Linux 包以交叉编译与包结构检查为准，是否完成实机加载另记于验证结果。
