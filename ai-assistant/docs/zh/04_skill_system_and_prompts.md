# 4 04 Skill 系统与提示词

Skill 是身份、指导和工具权限声明，不是工具实现。

## 4.1 分层

| kind/layer | 语义 |
|---|---|
| system Skill | Runtime 固定行为约束，如 thinking、Workflow Editor、Agent Test。 |
| role Skill | Agent 身份和职责；一个 Agent 必须且只能有一个 role。 |
| feature/capability Skill | 可渐进加载的业务能力，可有多个。 |

`SKILL.md` frontmatter 的核心字段是 `name`、`description`、`kind`、`tools` 和
`workflows`。正文是注入模型的执行指导。

## 4.2 工具白名单

```text
system tools + role tools + active feature tools
  -> ACTIVE_TOOLS
  -> state/tool_filter
  -> prompt tools section
  -> EXEC validation
```

只有被当前 Skill 集合引用的工具才对 Agent 可见。外部 RPC endpoint 可以暴露多个
工具，但 Skill 引用的是具体工具名，不是 endpoint id。

## 4.3 渐进加载

`GetSkillsList` 只返回可用 feature Skill 和激活状态。`UpdateSkills` 对 imported
feature Skills 使用全量替换语义，不是追加；main/system/role Skills 不会被替换。
更新后 Runtime 重新计算 `ACTIVE_TOOLS`。

## 4.4 Prompt 上下文

thinking 组合以下材料：

```text
system instructions
+ role persona and immutable role appendix
+ immutable cache entries
+ active feature Skill instructions
+ active tools/workflows
+ agent-scoped host dynamic text
+ retrieval context
+ current plan
+ ledger-derived history
+ runtime state/recorder context
```

`immutable_cache` 是实例创建时的一次性高优先级上下文输入，只在创建瞬间读取、合并
和注入；创建完成后不存在更新。宿主动态快照用于运行期间会变化的纯文本状态，两者
不可混用。动态快照按 `(conversation_id, agent_id, field_name)` 更新，字段值进入
prompt；恢复后由宿主重新发布。

## 4.5 Retrieval

检索配置属于 Agent。启用后，thinking 在每个用户回合首次推理前调用当前 Agent 的
检索端点并写入 `RETRIEVAL_CONTEXT`。若 Skill 白名单包含配置的 `tool_name`（默认
`RagRetrieve`），模型还可以显式二次召回。检索结果和动态快照都不是 role 身份。

下一篇：[05 Agent 与持久化](05_agent_and_persistence.md)
