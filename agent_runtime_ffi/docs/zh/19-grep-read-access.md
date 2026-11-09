# Grep 与 GrepJson 的集群和会话读取授权

宿主注册磁盘根目录，集群限定可授权的根目录 ID，宿主再按项目归属给具体会话授权子目录。Grep 与 GrepJson 在每次读取、目录枚举和分页时共用这套检查。AI 参数中的身份和磁盘路径不能授予权限。

## Runtime 资源注册

在现有资源注册文档中配置以下字段。

```json
{
  "grep": {
    "require_conversation_grants": true,
    "roots": [
      { "id": "project-data", "path": "E:/app-data/projects" },
      { "id": "attachments", "path": "E:/app-data/attachments" },
      { "id": "public-data", "path": "E:/app-data/public" }
    ]
  }
}
```

`require_conversation_grants` 默认是 `false`，兼容已有的全局根和 `conversation_grants` 文件接入。开启后，必须提供集群范围和会话 API 授权，缺失授权时返回 `read_access_required`，不会回退到全局根或旧授权文件。

## 集群范围

在现有集群注册文档中增加 `grep_root_ids`。

```json
{
  "schema": "agent-runtime-agent-cluster-registration/v1",
  "id": "workflow-editor",
  "description": "工作流编辑员工",
  "focus_agent_id": "primary",
  "agents": [{ "id": "primary", "role": "workflow-editor-role" }],
  "grep_root_ids": ["project-data", "attachments"]
}
```

集群只能选择资源注册中的根目录。提供该字段会要求会话 API 授权，即使全局兼容开关仍是 `false`。空数组表示不允许读取任何根目录。严格模式中未提供该字段时，可授权范围为空。集群更新继续遵守现有的存活 Conversation 占用限制。

## 会话授权

先创建会话，再完成授权，最后开放输入或恢复执行。宿主用普通 SDK `invoke` 调用以下命令。

```json
{
  "type": "conversation.set_read_access",
  "payload": {
    "conversation_id": "conv_123",
    "grants": [
      { "root_id": "project-data", "subpath": "workflow_abc" },
      { "root_id": "attachments", "subpath": "conv_123" }
    ]
  }
}
```

授权按整批替换，不与旧列表合并。空列表撤销所有根目录。返回包含 `conversation_id` 和已接受的 `grants`。任一候选无效，原授权保留。

`subpath` 使用相对路径和正斜杠，允许多层目录。目录须已存在，不能包含父级跳转、隐藏目录、符号链接或 Windows junction。空字符串表示明确授权该根的整个范围。每个 `root_id` 在一批授权中只允许出现一次，需要分别授权的项目资料、附件和公共资料可注册成不同根目录。

撤销使用以下命令。

```json
{
  "type": "conversation.revoke_read_access",
  "payload": { "conversation_id": "conv_123" }
}
```

两个接口仅用于宿主 FFI，没有注册成 AI 工具。Runtime 使用自己绑定的会话与集群身份，宿主不能通过授权请求扩大集群范围。关闭后继续授权会失败。功能探测使用 `features.conversation_read_access == 1` 以及命令目录。

## 生命周期与并发

会话授权只存在于内存，绑定到当前会话实例。新建、重建、从快照创建的会话需要重新授权。向已有会话导入快照也会清除 API 授权，宿主应完成重新授权后再执行任务。授权不会从旧会话文本、模型缓存或恢复文件中取得。

执行检索时持有授权读锁，授权更新与撤销使用写锁。接口返回前，已经使用旧授权的读取会结束，后续读取采用新授权。已读出的结果仍属于此前的执行记录。

授权只是读取范围，不会启用工具。宿主仍需在 Role／Feature Skill 中开放 Grep、GrepJson。Word、PDF 等格式解析、可检索文本或 JSON 的生成及版本管理继续由宿主完成。

该能力当前用于 Native Runtime。浏览器 WASM 没有本地磁盘根目录能力。
