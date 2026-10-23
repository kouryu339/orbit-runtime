# 动态集群测试包更新说明

基础版本为 0.5.1，来自 `jiangrong/workflow-logic-json` 分支。本地 Windows 测试包，未发布到远端，也未替换正式版。

- 新增 `runtime.add_agent_cluster`：Runtime 启动后新增集群配置，主 Agent 必须绑定已登记的有效 Role Skill。
- 新增 `runtime.update_agent_cluster`：提交完整配置及 `expected_version`。集群存在任意存活 Conversation 时禁止更新，包含等待、暂停、审批和后台状态；全部关闭后才允许更新。
- 占用错误返回集群 ID 与全部占用 Conversation ID；检查与提交和 spawn/close 共用现有 FFI 写锁。
- 集群目录和操作结果增加配置内容版本；新导出的集群会话快照保存该版本，恢复旧配置的归档时报告版本冲突。
- Rust、Python、Go、C++ SDK 已提供对应方法。详细载荷与兼容范围见 `docs/zh/13-dynamic-agent-clusters.md`。

本包保留 Workflow JSON 的六个对外操作、Schema、基本语法说明和示例。原有脚本接口和两个独立 Studio 页面仍不可用；thinking-pro 基本语法仍直接内嵌 DLL。

集群配置文件由宿主保存并在重启后重新登记。更新接口新增可选 `reload_skills`，在无存活 Agent 依赖时重读指定 Role／Feature 的元数据和正文；占用错误返回 Conversation 与 Agent ID，失败保留已接受的配置与缓存。文件由宿主原子保存，Runtime 不写回或回滚文件。快照记录 Skill 内容版本，恢复时拒绝不一致的正文。底层执行引擎没有改动。

请显式加载包内 `bin/agent_runtime.dll` 并使用包内 SDK。SDK 自动下载仍指向原正式发布版本，不能取得此本地构建。验证记录和 SHA-256 校验文件随 ZIP 邻接提供。

后续源码新增：`runtime.read_skill`、`runtime.validate_skill_transaction`、`runtime.commit_skill_transaction` 直接接收候选正文，并支持启动后新增 Role／Feature、内容版本校验、占用拒绝和单一清单事务恢复。受管集群可随事务持久保存。接口详见 `docs/zh/14-skill-candidate-transactions.md`。

工作流已有身份修改成功自动保存为草稿；正在执行的根工作流及其递归依赖禁止修改。草稿依赖的正式调用方保留但暂停正式执行，重新转正后恢复。请通过 capabilities.features 检测支持，不能仅凭 0.5.1 基础版本判断。
