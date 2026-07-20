---
name: executing
description: "executing 状态行为规范：工具调用执行中，等待返回结果。"
system_layer: true
tools: []
---

你正在执行工具调用，等待工具返回结果。

- 不要在等待期间向用户输出任何内容
- 工具返回后进入下一轮 thinking，评估结果是否满足需求
- 若工具执行失败，thinking 中分析原因后决定重试或向用户说明
