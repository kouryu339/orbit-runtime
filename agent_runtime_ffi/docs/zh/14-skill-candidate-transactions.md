# Skill 候选正文事务

员工的身份由 Role Skill 定义，Feature Skill 提供附加能力。这些是宿主 FFI 接口，不自动注册成 AI 工具。宿主应将制作会话绑定当前员工，只提交当前项目的候选，不给 AI 任意路径或通用 invoke 能力。

## 接口

| 命令 | payload | 行为 |
|---|---|---|
| `runtime.read_skill` | `{"name":"employee_role"}` | 读取已接受正文、元数据、内容 `version` 与 `store_revision`；名称不存在时返回 `exists:false`、`version:null` |
| `runtime.validate_skill_transaction` | 下述事务对象 | 校验候选、版本、工具、引用及占用；不写文件、不发布缓存、不执行 |
| `runtime.commit_skill_transaction` | 同一事务对象 | 全部成功才提交；返回整批已接受正文、元数据、内容版本和可选集群定义 |

SDK：Rust/Python 为 `read_skill`、`validate_skill_transaction`、`commit_skill_transaction`；Go 为 `ReadSkill`、`ValidateSkillTransaction`、`CommitSkillTransaction`；C++ 为 `read_skill`、`validate_skill_transaction_json`、`commit_skill_transaction_json`。使用现有 `agent_runtime_invoke_v1`，无新增 C 符号。JSON Schema 为 `schemas/skill-transaction.schema.json`。

## 新增员工身份及集群

Runtime start 后先读取目标名称，取得当前 `store_revision`。下例是 `runtime.commit_skill_transaction` 的 payload；外层仍使用现有 `agent-runtime-command/v1` 信封：

```json
{
  "expected_store_revision": 0,
  "skills": [{
    "name": "customer_service_role",
    "kind": "role",
    "expected_version": null,
    "source": "---\nname: customer_service_role\ndescription: 处理订单查询，不擅自退款。\nkind: role\n---\n\n负责解释订单状态，缺少订单号时先询问；退款须获得用户授权。\n"
  }],
  "cluster": {
    "expected_version": null,
    "registration": {
      "schema": "agent-runtime-agent-cluster-registration/v1",
      "id": "customer-service",
      "description": "客服员工执行集群",
      "focus_agent_id": "primary",
      "agents": [{"id":"primary", "role":"customer_service_role", "features":[]}]
    }
  }
}
```

`expected_version:null` 或省略表示新增；已存在则报内容版本冲突。修改必须传读取时的 Skill 内容版本。集群同理，新增传 null，修改传当前 `cluster.version`。`expected_store_revision` 对整份受管目录做并发控制；不同 Skill 并发保存也可能要求重新读取后重试。`skills` 可为空，但必须同时提供集群；这样可持久更新已受管集群而不改正文。

名单不可重复；名称只允许 1..128 个 ASCII 字母、数字、下划线和连字符。请求不接收保存路径。完整 `SKILL.md` 的 name/kind 必须和显式字段一致，description 与正文非空，kind 只接受 role/capability。系统、内嵌 Skill 不可覆盖，已有 kind 不可改变。工具必须存在于已安装目录，关联工作流必须是可正式执行的工作流。整批最多 64 份、总计 4 MiB，单份最多 1 MiB（UTF-8 字节）。

## 占用和失败

更新前检查进程内所有实际装填该 Skill 的存活 Agent，覆盖跨集群、动态 Feature、临时与后台 Agent；等待输入、暂停、审批仍算存活。新增不占用已有 Skill。提交集群配置时，该集群必须没有存活 Conversation。

占用检查、实例装填、恢复和缓存发布使用共享并发门；LLM 请求期间不持门。版本不符或占用时不自动关闭会话，候选由宿主继续保留。

| `error.kind` | details |
|---|---|
| `skill_in_use` | dependencies：conversation_id、agent_id、skill_names |
| `skill_version_conflict` | name、expected、actual（不存在时 null）；快照恢复也使用该错误 |
| `skill_store_conflict` | expected、actual；其他 handle/进程已更新目录时应重新打开资源，避免静默装填旧缓存 |
| `cluster_in_use` | cluster_id、conversation_ids |
| `cluster_version_conflict` | cluster_id、expected、actual |
| `invalid_argument` / `internal` | 格式、契约、写入失败说明 |

校验失败、写入失败或版本冲突保留上次已接受的文件、正文缓存和集群配置。成功才更新目录版本。宿主通过返回结果或 read_skill 更新列表，不先覆盖生效文件再请求重载。

## 存储及重启

资源登记的 `skills.managed_dir` 可指定独立可写目录，例如应用数据下的 `employees/runtime-store`；默认是 `skills.root_dir/.orbit-managed`。这是宿主启动配置，事务参数不能更换。不要将其放在只读安装目录。启动时从同一目录恢复已接受 Skill 与事务中提交的集群；无需再次 add 集群。

受管目录包含 `objects/<source-sha256>/SKILL.md` 和 `catalog.json`。先写入并刷盘不可变正文对象，再原子替换单一清单；清单同时保存元数据版本、整份源文档和完整生效集群。清单替换是唯一提交点：替换前进程中断仍恢复旧版本，替换后中断恢复整批新版本。暂存或未被清单引用的对象不生效，历史对象保留供诊断，不自动删除。提交成功后的清单和对象不应手工修改；损坏时拒绝加载，不静默回退。跨进程文件锁串行化写入，版本检查防止旧实例覆盖新目录；不提供跨进程存活 Agent 占用共享。

首次将已有文件 Skill 纳入事务时，Runtime 在受管目录保存新版正文覆盖层，保留原文件及其 reference/scripts 资源基目录；后续以清单版本为事实来源。宿主不得继续编辑原文件并期待覆盖层自动变化。新增 Skill 的资源基目录位于受管正文对象目录，本接口不上传额外资源文件。运行时同一进程的多个 handle 必须使用一致资源根；已有 Agent 时整体替换共享目录会拒绝。

事务只保存 Skill 和可选集群定义。员工显示名、分类、候选缓冲和其他宿主业务字段仍由宿主管理。存活实例拒绝身份更新，新会话读取成功提交的版本；快照绑定实际 Skill 内容版本，旧正文归档恢复明确报冲突。

原 `reload_skills` 继续兼容未受管的旧文件。受管 Skill 与事务登记的集群必须使用本接口，防止重载旧文件或仅修改内存后重启丢失。

capabilities 的 `features.skill_candidate_transactions` 为 `1`；不能仅用基础版本号判断本地 DLL 是否具备此接口。
