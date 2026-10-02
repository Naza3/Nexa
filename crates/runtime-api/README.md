# runtime-api

T04 纯 Rust 本机管理与文本 HTTP API；不链接 engine-host/llama-adapter/native。接口以[执行规格](../../ai-runtime-v0.1-execution-spec.md#7-http-与客户端契约)及[ADR0005](../../docs/decisions/0005-t04-loopback-http-and-management.md)为准，实施状态见[当前状态](../../PROJECT_STATE.md)。

- 仅回环、精确 Host、默认拒绝 Origin、Bearer；健康正文仅存活
- Chat 文本子集、严格未知/重复字段、统一错误；Started 后才正常 SSE
- 管理安全摘要分页64/128；Store导入依靠actor原子预约，查询/取消独立blocking边界
- 96KiB完整非流式上限；SSE与非流式共享core/IPC输出信用，不截断/落盘/重放
- HTTP/Ctrl+C/启动失败统一ServiceShutdown，清理未确认保持错误
- status/devices未知native指标为null/unavailable，不能将配置伪报实测

Config保留默认context4096；当前矩阵使用显式2048、threads2、batch128、cpu。此crate中的NoInference/fake仅用于契约测试。实际模型链路与Windows结果另行记录，不从单测推导模型可用性。


## 外部模型库接入（T06）

/runtime/models仍兼容旧游标请求，新增generation UUID；新客户端后续页传同一generation，变化返回model_list_changed。ModelSummary新增storage与availability_error；/runtime/status新增实际model_library身份和selected_model_display_name，不从磁盘新配置假装运行实例已切换。

load和/v1/chat/completions的隐式首次load共用外部准备路径：独立300秒协作核验预算、storage permit、RegistryLease和可取消blocking任务。断流/关停取消准备但不提前释放lease；失败更新可用状态及列表generation。现有客户端750秒响应头/请求等待与native load deadline不增加。wait_shutdown仅在runtime/worker确认关闭后释放external source guard，所有错误原样保留。
