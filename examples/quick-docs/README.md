# 快速文档 / Quick Documentation

五个独立静态 HTML，通过架构图、时序图和简化伪代码介绍 Orbit Runtime 的核心机制。
内容以 **v0.5.1 / `a8b9270`** 的实现为基线，使用简体中文；精确的公开契约以
[Runtime 契约文档](../../agent_runtime_ffi/docs/README.md)为准。

## 阅读顺序

| 页码 | 文档 | 主要内容 |
| --- | --- | --- |
| 01 | [Agent 核心循环](agent-core-loop.html) | 状态流转、模型调用前的轮数与 token 预算、工具执行与停驻。 |
| 02 | [会话暂停与恢复](session-pause-resume.html) | 两种暂停模式、在途调用、快照恢复与安全重放。 |
| 03 | [工作流引擎抽象设计](workflow-engine-design.html) | 图定义、节点与引脚、Exec / Data 双流、执行上下文与 Trace。 |
| 04 | [交互回传与事件推送](interaction-callback-sse.html) | 命令检查、工具审批、Widget、事件队列与宿主 SSE。 |
| 05 | [外部 RPC 工具](external-rpc-tools.html) | 工具发现与注册、Execute 双向流、HostCall、结果与隔离边界。 |

克隆或下载仓库后，用浏览器打开任意一个 HTML，通过页内导航切换。
五页均内嵌样式与 SVG，不依赖外部字体、脚本或服务，可以离线阅读；小屏可横向滚动图示。
GitHub 文件页显示 HTML 源码，下载后在浏览器中打开即可查看排版。

These five self-contained HTML pages are visual quick documentation in Simplified
Chinese. Clone or download the repository and open any page in a browser. No build
step, external assets, or network connection is required.

## 项目示例 / Project Examples

- [assistant-tauri](https://github.com/kouryu339/assistant-tauri)：独立的桌面宿主参考应用。
- [Reidom](https://github.com/kouryu339/reidom)：AI 原生 MIDI 编曲应用，结合 Tauri UI、
  JUCE 工程与音频引擎、Agent Tool 服务，以及 Orbit Runtime 配置和 Skills。

项目示例的构建方式、依赖版本与运行要求以各自仓库为准。

## 继续阅读

- [产品集成指南](../guides/README.md)
- [Runtime 命令、事件与恢复契约](../../agent_runtime_ffi/docs/README.md)
- [Runtime Host 与 RPC Tool SDK](../../sdk/README.md)
