# v0.5.0 相对 v0.4.8 的版本更新报告

- 上一个正式版：[v0.4.8](https://github.com/kouryu339/orbit-runtime/releases/tag/v0.4.8)，提交 `946c1aa1fe9030a2ef2fec9d9d24eec4c29f69d4`。
- 本次正式版：[v0.5.0](https://github.com/kouryu339/orbit-runtime/releases/tag/v0.5.0)，发布提交 `67e3152072ba3a3a9862b673cf20a8cbab5047c5`。
- [两个正式标签的代码差异](https://github.com/kouryu339/orbit-runtime/compare/v0.4.8...v0.5.0)。
- 发布日期：2026-10-12（Asia/Shanghai）；原生 ABI 继续为 1.3。

本文按正式版之间的差异组织，不以中间 Beta 为基线。v0.4.8 已有的原生函数调用、Workflow 执行日志、Jev 管理、ToolDescribe、90% 压缩阈值和 Split 的 null 处理不重复算作本次新增。

## 版本变化

| 部分 | v0.4.8 | v0.5.0 的变化 |
|---|---|---|
| 会话消息 | 文本消息及工具调用历史。 | 新增有序文本/图片 parts、会话图片导入、图片引用的发送、保存与恢复；三种 API 范式编码对应图片内容。 |
| Lit 图片交互 | 没有本次新增的图片输入及引用入口。 | 支持选择和粘贴图片，在文字光标位置插入 @图片引用；消息内显示引用，同对象使用稳定颜色。 |
| 历史与诊断 | 已有可恢复压缩和 HistoryRead。 | 图片纳入预算估算，摘要及历史读取保留图片引用而不编造视觉内容；诊断不记录嵌入的图片数据。 |
| View 控件 | 支持内联控件，同时存在旧 Asking 指导及日期/时间类型。 | 删除旧 Asking 模板，保留 View 载体与标识；保留确认/取消、单选、多选、路径、文本五类控件，移除日期/时间。 |
| View 选择和提交 | 没有本次统一的自由输入与消息级提交/过期行为。 | 单选、多选自动附加“其他”输入；整行颜色表示选中；整条消息只有一个汇总提交入口，提交后及出现后续消息时禁用旧控件，并显示已提交/已过期状态。 |
| 长 JSON 读取 | Grep 按文本搜索，结构及父子关联需另行还原。 | 新增 GrepJson 的概览、结构搜索、完整子树读取和分页；返回 Pointer、父级/祖先信息、原始文件 revision 和继续位置。 |
| 默认工具开放 | 默认 Think 提供 Skill 发现/激活、Grep 和 Jev 工具。 | 默认移除 GetSkillsList、UpdateSkills、Grep、ListJev、RunJev；新增 GrepJson 也不默认开放，全部由宿主按 Skill 提供。工具实现/API 保留，HistoryRead 和 ToolDescribe 继续默认可用。 |
| 模型配置 | 没有本次新增的模型级 reasoning 配置。 | 新增每个模型的推理强度及范式附加字段，贯通注册、持久化、公开回读和流式/非流式请求；Lit 提供编辑入口。 |
| 思考协议状态 | Anthropic 未保留本次补齐的 thinking 块续传。 | 保留 Anthropic thinking/redacted_thinking 块并回传，覆盖原生工具与旧行式工具历史。 |
| SDK 与发布 | 下载 v0.4.8 原生包。 | Rust/Python/Go/C++ 增加图片导入调用；Runtime、Rust/Python 包版本及各语言下载元数据更新到 0.5.0，正式代码合入 main。 |

### GrepJson 的完整性与版本控制

GrepJson 与 Grep 共用宿主授权根和读取限制，不引入新的写入工具。合法特殊字段名、任意精度数值和 Unicode 保留；重复字段、错误语法、过深或超限输入明确拒绝。输出超限时返回完整性标记和可继续读取的路径，不截断成破损 JSON。

分页非零 offset 要求 expected_revision；文件变化返回 409。小对象在当前页装不下时整体留到下一页，减少额外子树读取。修改仍由宿主工具在共享写锁中检查 revision、验证业务 Schema 并原子写入。

### 推理设置仍按三个范式处理

| 所选范式 | 上游字段 |
|---|---|
| openai_chat_completions | reasoning_effort |
| openai_responses | reasoning.effort、可选 reasoning.summary |
| anthropic_messages | output_config.effort、可选 thinking.type / thinking.budget_tokens |

不新增厂商专用范式或参数映射。不配置 reasoning 时保持原有请求行为。显式推理设置会校验字段组合并处理不兼容的采样参数；Anthropic 固定预算与输出上限、强制工具选择的冲突在发送前返回错误。

## 从 v0.4.8 升级

1. 同步 v0.5.0 原生库、SDK 下载元数据和 Lit dist，重启 Runtime。C ABI 主版本不变，JSON 消息新增可选 parts 与模型 reasoning。
2. 宿主要使用图片时，提供 Lit 图片导入接口，把宿主可访问的图片路径交给 conversation.import_image，并原样传递消息的有序 parts。
3. 之前依赖默认 Think 激活 Skill、Grep 或 Jev 的宿主，应在自定义 Skill 中显式提供对应工具。GrepJson 同样需要显式开放和读取根授权。
4. 模型只提供单选/多选推荐项，不再自行补“其他”；由 Lit 提供输入。停止生成 input:date/input:time，按需改为普通文本输入。
5. 按实际兼容端点的能力设置 reasoning。详细字段见 [Provider 配置契约](../../zh/03-runtime-provider-config-contract.md)，长 JSON 使用见 [GrepJson 契约](../../zh/16-grep-json.md)。

## 发布验收与边界

- Rust 工作区串行测试、格式检查通过；Runtime 137 项、AI Assistant 348 项及 Gateway、SDK、JSON 验收测试通过。
- Lit 类型检查、74 项测试和构建通过；Go SDK、C++17 示例及 Python 元数据/语法检查通过。
- 2,360,802 字节 JSON 场景完整恢复 256 个关键帧，分页相关调用从 12 降至 8；旧 revision 拒绝，重新定位正确。
- 推理真实验收使用现有宿主 DeepSeek 凭据：38 个场景、41 次请求，0 失败。

| 真实接口 | 场景 | 通过 |
|---|---:|---:|
| DeepSeek Chat Completions | 13 | 13 |
| DeepSeek Responses | 13 | 13 |
| DeepSeek Anthropic Messages | 12 | 12 |

真实验收覆盖默认、low/high、额外支持档位、关闭思考、流式/非流式、完整工具往返和 thinking 回传。未改宿主配置，未调用业务写入工具，凭据及原始思考内容未进入源码或验收日志。可重复测试为 ai-gateway/tests/live_reasoning.rs，默认 ignored；显式提供 RUNTIME_REASONING_LIVE_CONFIG 和 RUNTIME_REASONING_LIVE_REPORT 后运行，extended 阶段测试额外档位。

- Windows DLL 的 0.5.0 版本、ABI 1.3、加载、注册、启动、配置回读、工具目录、关闭与销毁通过。
- Linux x86_64 交叉编译、链接、ELF 与打包检查通过；未执行 Linux 动态加载。
- 正式版四个 GitHub 附件的摘要/大小与本地包及 SDK 清单一致，发布状态为稳定版 Latest；旧 Beta 发布及标签已删除。

真实接口验收只覆盖 DeepSeek 的三个兼容端点，不代表 OpenAI/Anthropic 原厂均已实测。DeepSeek 会映射部分 effort 档位、忽略固定 thinking 预算，并接受但不生成 Responses 摘要。参数被接受不等于所有附加字段生效；简单验收题也不是推理质量或费用对比基准。本版只提供图片输入，不含音频、视频及生成图片输出。

## 发布包

| 平台 | ZIP 字节数 | SHA-256 |
|---|---:|---|
| Windows x86_64 | 14197921 | bd2c51edce8b555aacc3edde2f3640d391b852e91057890f8b8f58e7d419a952 |
| Linux x86_64 | 13984383 | 3b7180041d40486828eba57520454019d6890548206e8a917ebdaf26dd0d9cf6 |

每个平台另附 .zip.sha256 文件。下载及完整英文更新说明见 [正式发布页](https://github.com/kouryu339/orbit-runtime/releases/tag/v0.5.0) 和 [release-notes.md](release-notes.md)。

## Engineering Review

Architecture: 以两个正式版本的差异描述模块边界和宿主迁移。
Code: 仅列实际增量，不将 v0.4.8 已有功能重复算作新增。
Concurrency: 模型配置隔离、工具历史和 JSON 读改写边界均有验收依据。
Resource: 保留真实调用规模及 JSON 读取限制；报告不包含原始诊断数据。
Audit: 发布提交、标签、包哈希和真实验收范围明确。
Verification: 文档与正式标签差异、产物及已完成验收一致。

Risks: Linux 动态加载和 OpenAI/Anthropic 原厂未实测；兼容端点可能忽略附加字段。
Decisions Needed: 无。
