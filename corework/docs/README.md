# Corework 文档

当前文档按语言分目录维护：

```text
zh/
en/
```

`corework` 文档只描述 crate 内部能力：cache、事件、执行单元、状态机、workflow、节点注册、RPC tool 协议。Agent Runtime FFI、前端事件、宿主微服务接入不在这里重复维护，统一参考：

```text
../../agent_runtime_ffi/docs/zh/
```

## 中文文档

| 文档 | 内容 |
|---|---|
| [`zh/01_architecture.md`](./zh/01_architecture.md) | Corework 架构与模块边界。 |
| [`zh/02_cache_system.md`](./zh/02_cache_system.md) | Cache 抽象与使用方式。 |
| [`zh/03_decorators_and_registration.md`](./zh/03_decorators_and_registration.md) | 本地系统、节点和元数据注册。 |
| [`zh/04_blueprint_system.md`](./zh/04_blueprint_system.md) | Blueprint/workflow 结构。 |
| [`zh/05_execution_unit_module_and_state_machine.md`](./zh/05_execution_unit_module_and_state_machine.md) | 执行单元、模块、状态机。 |
| [`zh/06_business_development_guide.md`](./zh/06_business_development_guide.md) | 业务系统开发与工具接入。 |
| [`zh/09_rpc_tool_protocol_v1.md`](./zh/09_rpc_tool_protocol_v1.md) | 当前 RPC Tool gRPC 协议。 |

旧的 DLL/RPC 完成记录和 RPC 集成记录已经删除。正式 RPC 说明以 `09_rpc_tool_protocol_v1.md` 和 `proto/corework_agent_tool_v1.proto` 为准。
