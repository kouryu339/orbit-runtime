# Workflow JSON 更新公告

本地构建：`jiangrong/workflow-logic-json`，Runtime/SDK 基础版本 `0.5.1`。此包是分支验证包，未替换线上发布版。

## 接口变更

对外仅保留六个操作：

| Runtime 命令 | AI 工具 | 用途 |
| --- | --- | --- |
| `workflow.list` | `listWorkflows` | 只读列出资源摘要，支持 all/draft/registered 筛选 |
| `workflow.register` | `registerWorkflow` | 提交完整资源源文本；创建或按 revision 替换 |
| `workflow.remove` | `removeWorkflow` | 按 ID 和当前 expected_revision 移除 |
| `workflow.validate` | `validateWorkflow` | 静态验证，不落盘、不执行 |
| `workflow.promote` | `promoteWorkflow` | 草稿转正，校验 expected_revision |
| `workflow.execute` | `executeWorkflow` | 按资源 state 真实执行 |

旧版脚本统一不可用。旧脚本编辑、编译、转换、读写及直接执行接口均不再对外开放，调用返回 `deprecated_api`。旧 register 的 id-only 请求也不再接受。SDK 中旧方法仅保留废弃声明，不代表 Runtime 支持旧请求。

Workflow Studio 与 Agent Test Studio 两个独立页面已弃用，打开页面的命令已关闭；本轮不迁移这两个页面。

## 保存和启动

- 源文档包含 `schema/id/metadata/state/revision/logic`。`state` 是 `draft` 或 `registered`，新建 revision 为 1；更新提交当前 revision，成功后 Runtime 递增。
- Runtime 将完整源文本保存在配置的工作流目录，新资源文件名为 `<id>.workflow.json`。裸表达式不是标准 JSON；请使用 Runtime 解析器。字符串、表达式、宿主扩展字段都保留原意。
- 启动时在安装工具契约之后自动读取目录中的 `.json`、`.workflow` 资源文档，按正式工作流引用关系编译；草稿和正式资源、revision 均恢复。无需宿主再次逐一注册。
- 旧脚本及旧图资源不会自动加载或迁移，原文件保留并记录跳过原因。请用新格式重新登记。内部图编译与执行机制继续沿用原核心。
- 无效的新资源、重复身份、缺失引用或引用环会使启动失败并报告原因；启动检查不会执行节点或覆盖源文件。
- 注册失败保留上一版本。正式接口变更若破坏调用方则拒绝；执行开始时固定本次图和引用快照。布局编辑与独立画布迁移不在本轮范围内。
- 移除会删除 Runtime 管理目录中的对应源文件，包括手动放入该目录的资源。请将作者备份放在管理目录之外。

## 使用本地包

直接加载包内 `bin/agent_runtime.dll`，并使用包内 `sdk/` 的接口。SDK 默认下载地址仍指向原正式 0.5.1，自动下载不会取得本分支构建。

包内包含两份 Schema、可编辑示例、接口说明及各语言 SDK 源码。Schema 校验针对 Runtime 解析后的普通 JSON 结构，不能用标准 JSON 解析器直接读取带裸表达式的源文档。

控制字段 `if/for/switch/set.value` 本轮仍使用结构化表达式；裸引用与 Pure 调用开放在任意节点 `args` 值及 `returns.<name>.value` 内。变量必须预先声明，循环默认提供当前层 `item/index`，switch 的 cases 数量可变。

## AI 语法说明更新

thinking、thinking-pro 及其原生工具调用别名不提供脚本工具。提示词按当前 JSON 注册／验证能力注入新的资源源语法，不再依赖废弃的脚本执行工具。中英文说明仅描述字段、常量和引用、Pure 调用形式及作用域；不预置或自拟节点、函数和引脚，具体契约按需查询运行时。

基本语法说明现在包含一个可提交的最小完整示例（仅输入、预声明变量、赋值、返回），并明确 schema 值仅为版本标识；Schema 负责结构校验，不替代基本写法说明。
