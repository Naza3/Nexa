# runtime-api

T04 纯 Rust 本机管理与文本 HTTP API；不链接 engine-host/llama-adapter/native。接口以[执行规格](../../ai-runtime-v0.1-execution-spec.md#7-http-与客户端契约)及[ADR0005](../../docs/decisions/0005-t04-loopback-http-and-management.md)为准，实施状态见[当前状态](../../PROJECT_STATE.md)。

- 仅回环、精确 Host、默认拒绝 Origin、Bearer；健康正文仅存活
- Chat 文本子集、严格未知/重复字段、统一错误；Started 后才正常 SSE
- 管理安全摘要分页64/128；Store导入依靠actor原子预约，查询/取消独立blocking边界
- 96KiB完整非流式上限；SSE与非流式共享core/IPC输出信用，不截断/落盘/重放
- HTTP/Ctrl+C/启动失败统一ServiceShutdown，清理未确认保持错误
- status/devices未知native指标为null/unavailable，不能将配置伪报实测

Config保留默认context4096；当前矩阵使用显式2048、threads2、batch128、cpu。此crate中的NoInference/fake仅用于契约测试。实际模型链路与Windows结果另行记录，不从单测推导模型可用性。
