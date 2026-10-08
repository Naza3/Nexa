# runtime-api

T04 纯 Rust 本机管理、文本与单图 OCR HTTP API；不链接 engine-host/llama-adapter/native。接口以[执行规格](../../ai-runtime-v0.1-execution-spec.md#7-http-与客户端契约)及[ADR0005](../../docs/decisions/0005-t04-loopback-http-and-management.md)为准，实施状态见[当前状态](../../PROJECT_STATE.md)。

- 管理监听仅回环、精确 Host、默认拒绝 Origin、Bearer；健康正文仅存活
- Chat 文本子集、严格未知/重复字段、统一错误；Started 后才正常 SSE
- 管理安全摘要分页64/128；Store导入依靠actor原子预约，查询/取消独立blocking边界
- 96KiB完整非流式上限；SSE与非流式共享core/IPC输出信用，不截断/落盘/重放
- HTTP/Ctrl+C/启动失败统一ServiceShutdown，清理未确认保持错误
- status/devices未知native指标为null/unavailable，不能将配置伪报实测
- 本机 `GET /runtime/performance` 返回真实服务 instance_id 和 actor 最近 200 条推理终态；聊天/OCR/LAN 共用记录，LAN 不开放管理查询。详见 [ADR0035](../../docs/decisions/0035-unified-inference-performance.md)

Config保留默认context4096；当前矩阵使用显式2048、threads2、batch128、cpu。此crate中的NoInference/fake仅用于契约测试。实际模型链路与Windows结果另行记录，不从单测推导模型可用性。


## 外部模型库接入（T06）

/runtime/models仍兼容旧游标请求，新增generation UUID；新客户端后续页传同一generation，变化返回model_list_changed。ModelSummary新增storage与availability_error；/runtime/status新增实际model_library身份和selected_model_display_name，不从磁盘新配置假装运行实例已切换。

load和/v1/chat/completions的隐式首次load共用外部准备路径：独立300秒协作核验预算、storage permit、RegistryLease和可取消blocking任务。断流/关停取消准备但不提前释放lease；失败更新可用状态及列表generation。现有客户端750秒响应头/请求等待与native load deadline不增加。wait_shutdown仅在runtime/worker确认关闭后释放external source guard，所有错误原样保留。

## 可选 LAN 推理监听（ADR0020）

独立 `lan_api` 配置默认 disabled、listen 为空、allowed_cidrs 为空；旧配置仍仅本机监听。显式启用只支持具体 RFC1918 IPv4 和非零端口，1–16 条 canonical /24–/32 私网 CIDR，无重复、重叠或主机位；IPv6（含 mapped 地址）拒绝。

LAN router 仅开放 GET /v1/models 和 POST /v1/chat/completions；models 只观察当前已加载模型。chat 通过 actor 的 submit_loaded 原子准入，保留共享 FIFO，绝不扫描/hash、加载、切换或重载模型；未加载/空闲卸载后返回 model_not_loaded。所有 /runtime、health、proof 和其他方法均不可用；X-Request-ID、Origin、Forwarded/X-Forwarded-* 和 X-Real-IP 拒绝，验证真实 socket 两端和精确 Host。只使用独立 LAN Bearer，不接受 cookie/query 认证。

可信 LAN HTTP 没有 TLS 机密性；不支持公开服务/反代身份，不改防火墙或 NAT。两监听共享 64 连接上限，LAN 至多48以为本机保留16；请求/队列/输出/取消预算沿用，LAN body 另有10秒总读取截止。两端点预绑定成功后才创建运行时/发布本机发现；任一服务失败触发共同关停并等待两端连接/worker清理。

LAN 专用凭据仅显式 enabled 的服务启动生成至安全 secrets/lan-api-token；本机 init/浏览/status 不生成，配置/status/discovery 不含密钥。实际 running 观测与 saved enabled 分开。测试的真实 TCP 仅用 loopback，加测试层模拟私网 peer；不能当作 Windows 网卡/防火墙/两设备验收。

手动停止加载按[ADR0027](../../docs/decisions/0027-owned-model-load-cancellation.md)：每次操作独立UUID/取消令牌，取消ACK与清理终态分离，覆盖准备/切换/加载及自有短测，不影响其他客户端或全服务。

## 本机单图 OCR

按 [ADR0032](../../docs/decisions/0032-local-single-image-ocr.md)，Chat Completions 可接收一个 user 的 image_url data URL 与 text，PNG/JPEG 文件≤4 MiB、本机封套≤8 MiB；普通文本/管理与LAN保持≤1 MiB，LAN拒绝图像。双文件导入在原管理import中增加 `projector:{file,expected_sha256?}`，摘要含 `has_projector` / `projector_size_bytes`；配对加载仅报告Loaded而非文本Passed。原取消、SSE和安全边界复用。
