# 8 Runtime 诊断日志契约

日志给人排查问题，事件给机器消费状态。宿主、前端、状态镜像器应消费
`agent-runtime-event/v1`，不应解析 runtime 日志文件来同步状态。

## 8.1 默认日志

默认运行只保留低噪声、可定位问题、不会持续泄露上下文的日志。

| 类别 | 默认级别 | 内容 | 不包含 |
| --- | --- | --- | --- |
| 启动与配置摘要 | `info` | runtime instance id、profile id、data_dir、skills_dir、provider config 加载状态 | API key、完整 provider JSON、完整 prompt |
| 生命周期 | `info` | runtime start/stop、conversation create/close/import/materialize 的 id 和结果 | 用户消息原文、完整 snapshot |
| 外部依赖错误 | `warn/error` | LLM HTTP 状态码、错误码、模型 id、重试次数、简短错误摘要 | request body、response body 全文、认证头 |
| 事件出口健康 | `warn/error` | event envelope 构造失败、handle 队列断开 | 每条正常事件全文 |
| 工具执行异常 | `warn/error` | tool name、call id、错误码、耗时、失败摘要 | 工具参数全文、工具结果全文 |
| 恢复/路由异常 | `warn/error` | not_owner、runtime unavailable、materialize/rebind 失败、route lease busy | 历史 ledger 全文 |

## 8.2 显式诊断

以下内容只能在显式诊断开关下输出：

| 内容 | 原因 | 开关 |
| --- | --- | --- |
| LLM messages、prompt、history、compact 后上下文 | 可能包含用户隐私且体积大 | `RUNTIME_CONTEXT_PROBE=1` |
| 动态 snapshot 原文 | 可能包含页面状态、用户输入、业务对象 | `RUNTIME_CONTEXT_PROBE=1` |
| LLM request/response 详情 | 可能包含 prompt、工具 schema、模型输出 | `RUNTIME_LLM_TRACE=1` |
| 每条 `frontend:state_snapshot` 完整 payload | 这是事件，不是日志 | 通过 `next_event_v1` 消费 |
| LLM 用量/错误事实 | 持久化计量与诊断 | 消费 `conversation.ledger_delta` 中 `metadata.subtype = "llm_usage"` 或 `"llm_error"` 的记录 |
| MQ/SSE 正常 publish 成功日志 | 高频无诊断价值 | 只记录失败或采样指标 |

## 8.3 环境变量

推荐默认值：

```bash
RUST_LOG=info,agent_runtime_ffi=info,ai_assistant=info,llm_gateway=warn,corework=warn
RUNTIME_LLM_TRACE=0
RUNTIME_CONTEXT_PROBE=0
AI_GATEWAY_DIAGNOSTICS=off
```

排查 LLM 请求链路时：

```bash
RUNTIME_LLM_TRACE=1
RUNTIME_LLM_TRACE_FILE=./data/logs/agent-runtime-llm-trace.jsonl
```

排查上下文污染、compact 错误或动态 snapshot 注入错误时：

```bash
RUNTIME_CONTEXT_PROBE=1
RUNTIME_CONTEXT_PROBE_FILE=./data/logs/runtime-context-probe.log
```

## 8.4 文件职责

| 文件/出口 | 责任 |
| --- | --- |
| `{data_dir}/logs/agent-runtime.log` | runtime diagnostics：启动、配置摘要、关键错误 |
| `RUNTIME_LLM_TRACE_FILE` | LLM 请求链路诊断 JSONL，仅显式开启 |
| `RUNTIME_CONTEXT_PROBE_FILE` | 上下文探针，仅显式开启 |
| FFI pull queue | `agent_runtime_next_event_v1` 返回 `agent-runtime-event/v1` |
| 宿主 stdout/stderr | Go/Python/Node 宿主服务日志，由宿主进程管理器采集 |

## 8.5 日志与事件边界

对外稳定协议：

```text
agent-runtime-event/v1
```

内部诊断文件：

```text
agent-runtime.log
agent-runtime-llm-trace.jsonl
runtime-context-probe.log
```

宿主可以把 pull queue 事件转成 SSE、Redis Stream、Kafka、RocketMQ 或其他传输；
不应 tail runtime 日志来实现状态同步。
