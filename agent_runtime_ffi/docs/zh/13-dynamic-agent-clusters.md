# 动态集群配置

员工正文新增／保存使用 [Skill 候选事务](14-skill-candidate-transactions.md)，可同时提交集群；受管集群重启自动恢复。本文的 `reload_skills` 仅保留未受管旧文件兼容，不能替代候选事务。

会话以集群为创建单位。`agents` 是集群成员定义，`focus_agent_id` 指定主 Agent；主 Agent 必须绑定一个有效、正文非空且 `kind: role` 的 Role Skill。Feature Skill 是附加能力，不能充当主身份。可以直接配置成员，也可以通过现有 `profile` 引用默认配置。

## 新增

Runtime 启动后调用 `runtime.add_agent_cluster`，沿用现有集群配置形状：

```json
{
  "schema": "agent-runtime-command/v1",
  "type": "runtime.add_agent_cluster",
  "payload": {
    "registration": {
      "schema": "agent-runtime-agent-cluster-registration/v1",
      "id": "customer-service",
      "description": "客服集群",
      "focus_agent_id": "primary",
      "agents": [{
        "id": "primary",
        "name": "客服",
        "role": "customer-service-role",
        "features": [],
        "system_skills": {"thinking": "thinking-pro"}
      }]
    }
  }
}
```

`customer-service-role` 及 Feature 必须已存在于启动时加载的 Skill 目录中。这个请求不定义 Skill 正文、不创建会话，也不注册新的工具端点；启动后才写入的新 Skill 文件不会被这两个接口自动注册。模型可以留空，沿用现有延迟配置规则；启用检索时，所需的本地检索工具必须已在启动时安装。

成功返回 `agent-runtime-agent-cluster-change/v1`，包含 `action: added` 和完整的生效集群定义 `cluster`，其中 `version` 是解析后集群配置的内容摘要。已有 ID、内置集群 ID、无效 Role／Feature、模型或配置字段都会被拒绝。

随后使用现有 `conversation.spawn` 创建会话：

```json
{"schema":"agent-runtime-command/v1","type":"conversation.spawn","payload":{"cluster_id":"customer-service"}}
```

每次 spawn 实例化集群的全部初始成员。不同会话之间的成员实例相互独立。

## 更新

`runtime.update_agent_cluster` 接收同一份完整 `registration` 和 `expected_version`：

```text
payload = {
  "registration": 完整集群配置,
  "expected_version": 上次新增或查询返回的 cluster.version
}
```

版本可从 `runtime.get_agent_cluster_definitions` 获取。更新表示替换指定 ID 的完整集群配置，包括成员、主 Agent、模型选择和工具权限。只要该集群还有任意一个存活 Conversation，就拒绝更新；执行、等待输入、暂停、等待审批、后台工作等状态均不豁免。关闭全部会话后才允许更新。

占用错误的 `error.kind` 为 `cluster_in_use`，`error.details` 包含 `cluster_id` 和所有占用的 `conversation_ids`。版本冲突为 `cluster_version_conflict`，包含期望版本和实际版本。失败时保留原配置、源文件及运行中的缓存。

占用检查和配置提交与 spawn/close 共用现有 FFI handle 写锁，不存在“检查后被新会话抢先加载”的窗口。更新影响后续创建的会话，不修改存活实例。配置内容不变时版本不变；摘要覆盖生效配置，不是递增计数器。

## 同时重载 Skill 正文

更新请求可增加可选字段 `reload_skills`，重读宿主已经保存的、已登记的 Role／Feature Skill 文件：

```text
payload = {
  "registration": 完整集群配置,
  "expected_version": 上次返回的 cluster.version,
  "reload_skills": ["customer-service-role", "order-query"]
}
```

名单不可重复，必须由本次提交的集群引用。重载保留 Skill 名称与 kind，正文必须非空，系统与内嵌 Skill 不允许替换。未提供该字段或传空数组只更新集群配置。

除目标集群不得有存活会话外，Runtime 还检查整个进程中存活 Agent 的实际装填引用，覆盖其他集群、初始成员、临时子 Agent、后台 Agent 和动态导入的 Feature。等待输入、暂停和等待审批仍算存活。只出现在配置中而未装填的引用不会占用 Skill。

被占用时返回 `error.kind: skill_in_use`，`error.details.dependencies` 列出 `conversation_id`、`agent_id` 和 `skill_names`。检查、引用装填及缓存发布通过同一个读写门协调，避免检查后又装填旧版的窗口；运行与 LLM 请求期间不持有这个门。全部文件校验通过后，集群配置、Skill 元数据与正文缓存一次生效；任一失败保留调用前的配置与缓存。成功结果的 `reloaded_skills` 返回每个 Skill 的内容版本。

文件由宿主保存，推荐先写临时文件再原子替换，完成后调用更新。Runtime 不写回或回滚宿主文件；直接编辑已缓存正文不会自动替换当前缓存。进程内锁不能约束外部编辑器。后续会话装填重载后的内容。

## 归档、文件与边界

- 新导出的集群会话快照携带 `cluster_definition`（cluster_id、version）和实际引用的 `skill_versions`。恢复和导入先检查配置与 Skill 内容版本；Skill 不一致返回 `skill_version_conflict`，不悄悄换正文继续。
- 经动态接口新增或更新的集群拒绝缺少版本的旧快照。既有启动前注册入口继续保留旧快照兼容行为；动态配置恢复时应在 start 后通过新增接口重新加载。
- 宿主保存集群配置文件，重启后重新登记。Runtime 不覆盖 `agent-cluster.json` 或 `resources.json`。相同生效配置在重启后得到相同版本摘要。
- `runtime.register_agent_cluster` 仍用于启动前注册，start 后保持冻结。Resources、工具端点和 LLM 注册边界不变。
- Skill 内容版本独立于集群配置摘要；仅重载正文不改变 `cluster.version`。动态集群拒绝缺少 `skill_versions` 的旧快照。已有存活 Agent 时，另一个 Runtime handle 也不能整体替换进程共享 Skill 目录。
- 内置集群不可通过动态接口覆盖；管理命令属于宿主 FFI，不增加 AI 工具。

SDK 提供 `add_agent_cluster`／`update_agent_cluster`（Go 为 AddAgentCluster／UpdateAgentCluster，C++ 为 add_agent_cluster_json／update_agent_cluster_json），通过已有 `agent_runtime_invoke_v1` 调用，无需新增 C 导出符号。
