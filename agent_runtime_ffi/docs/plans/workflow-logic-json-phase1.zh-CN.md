# Workflow JSON 实施状态

当前接口见 [资源契约](../zh/12-workflow-resource-api.md)，变更见 [更新公告](../workflow-json-update.zh-CN.md)。以下是当前落地结果。

- 源文档包含元数据、草稿状态、revision、输入、预声明变量和返回值。
- args 和 return.value 允许裸引用及 Pure 调用；其他控制字段仍使用结构化表达式。
- call/if/for/set/switch 复用原有 Chain AST、图编译器和执行器，switch 支持可变 cases 并编译为 Branch 链。
- 对外保留列表、注册、移除、验证、转正、执行；旧脚本 API 已关闭。
- Runtime 保存完整源文档，启动在工具安装后按引用依赖编译并恢复草稿、正式状态及 revision。旧资源跳过，失败不覆盖原文件。
- 接口更新保护正式调用方，运行固定引用快照。宿主扩展按命名空间保留。
- Workflow Studio 和 Agent Test Studio 两个独立页面已弃用，不迁移。
- 本轮仅打本地分支验证包，未修改线上版本、远程资产或 SDK 自动下载指向。

验证记录与包内实际 DLL 检查随本地构建报告提供。尚未实施 token 收益基准、任意画布编辑和完整动态类型证明，不作相关收益保证。
