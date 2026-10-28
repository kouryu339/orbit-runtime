# Orbit Runtime v0.5.2

本版合并 Workflow JSON 资源改造、Skill 候选事务、动态集群，以及 Lit/Svelte 对话前端更新。原生 ABI 继续为 1.3。

## Workflow（不兼容变更）

- 以 `orbit.workflow.resource/v1` 源文档登记工作流，Runtime 编译、验证、持久化，并在启动时自动恢复草稿和正式资源。
- 对外保留 `workflow.list/register/remove/validate/promote/execute`。旧脚本编辑、转换、编译、读写和执行接口统一不可用，返回 `deprecated_api`。旧脚本和旧执行图不自动加载或迁移，原文件保留；请按新格式重新登记。
- 修改正式工作流成功后转为草稿；执行中的工作流及完整引用链禁止修改。revision 检查、失败保留旧版本和执行快照保证继续生效。
- 源语法支持显式预声明变量、动态 switch 分支，以及 args/return 中的裸引用和 Pure 调用。带裸表达式的源文本需要 Runtime 解析器；标准 JSON Schema 校验解析后的结构。
- Workflow Studio、Agent Test Studio 两个独立页面弃用，对外打开接口关闭。

## 员工和 Skill

- 动态新增、更新集群配置，主 Agent 必须绑定 Role Skill；集群有任何存活 Conversation 时拒绝更新，包括等待、暂停和后台状态。
- 提供 `runtime.read_skill`、`runtime.validate_skill_transaction`、`runtime.commit_skill_transaction`，可在一个事务中提交 Role/Feature 候选正文和可选集群配置。
- 检查版本和正在使用的 Skill 依赖，失败保留原文件、缓存和版本；历史恢复对版本变化报告冲突。

## 对话前端

- Lit 与原生 Svelte 共用宿主、传输、内容投影与状态协议，Svelte 不嵌入 Lit 自定义元素。
- 统一模型和推理强度选择、图片与权限操作，以及发送、暂停、继续的状态控制。
- 补齐工具成功、失败、等待批准、执行请求与交互控件；用户界面显示用途和状态摘要，不展示工具命令、JSON 参数或原始返回值。
- 统一厂商弹窗、导入流程、白色主题、布局和按钮状态。

## 发布产物

- `orbit-runtime-runtime-v0.5.2-windows-x86_64.zip`
- `orbit-runtime-runtime-v0.5.2-linux-x86_64.zip`
- `orbit-runtime-frontend-svelte-v0.5.2.js`：直接使用的浏览器 ES Module。
- `orbit-runtime-frontend-svelte-v0.5.2.zip`：相同 JS（包内固定名称 `orbit-conversation-svelte.js`）、接入说明、文件校验清单和许可证。运行时、样式和字体内置，宿主负责传输接入。
- `orbit-runtime-sdk-v0.5.2.zip`：匹配此版本的各语言 SDK 源码及原生包下载校验信息。SDK 独立成包，避免原生包包含自身下载哈希的循环依赖。

各附件配有 SHA-256 文件。原生包包含 Workflow Schema、语法、示例、动态集群及 Skill 事务说明。将 SDK 包内 `sdk/` 放到原生包根目录，可使用附带的 Python 验证脚本。

前端摘要隐藏只涉及展示，宿主仍需控制传输与日志的访问权限。Svelte 单文件包含 Markdown、数学和图表依赖，建议宿主压缩传输并缓存。macOS 包本次不提供。
