# ADR0035：统一推理性能记录

2026-10-08，已采纳。任务 W02-PERF-1。用户要求 prefill/decode 速度及耗时，并明确覆盖所有推理。

## 归属与覆盖

计时来自原生执行线程，经可靠终态事件、私有 IPC、单 actor 传到本机查询接口。不使用 best-effort 的 ExecutionReport 日志通道，也不用前端字符数或到达间隔推测 token 速度。

actor 独占最多 200 条内存记录，最新优先；所有已接收 Generate 的 Completed/Cancelled/Failed 都在发布终态前记入，包括聊天、单图 OCR、本机和 LAN 的流式/非流式调用，以及实际进入生成的模型短测。HTTP 鉴权、DTO 或准入阶段拒绝的请求不属于推理记录。加载、卸载本身不生成一条测速。

记录包含模型/请求 ID、输入类型、接收时间、最终状态、输出上限、usage、调度耗时；成功且取得完整原生测量时附阶段耗时和实际加载参数。失败、取消、超时没有成功阶段指标。清理未确认仍优先标记 Failed；取消与完成竞态不保留成功指标。无输入、输出正文、图片、路径、密钥或错误全文，不新增磁盘数据库。

## 计时口径

| 字段 | 单位 | 范围 |
| --- | --- | --- |
| `prepare_us` | 微秒 | prepare：模板、token 化、图片解码/预处理和预算检查 |
| `prefill_us` | 微秒 | 原生 phase 0 至 phase 2；图片请求包含视觉编码与图文输入求值 |
| `decode_us` | 微秒 | phase 2 至生成调用返回，减去同步输出回调时间；仍含采样、文本处理及清理 |
| `output_callback_us` | 微秒 | 同步输出回调总时间，含复制、锁和信用等待，并非仅阻塞时间 |
| `queue_ms/load_ms/execution_ms` | 毫秒 | actor 原有的排队、模型加载、执行时间；执行包含 prepare/prefill/decode/交付等开销 |

prefill token/s = `usage.prompt_tokens * 1e6 / prefill_us`；decode token/s = `usage.completion_tokens * 1e6 / decode_us`。usage 沿用原生计数，包括首个采样 token/EOG，不按字符估算。阶段不完整、乱序或非法时指标为空；零分母速度不可用。这不是 llama-bench 的纯内核速度，图片/文本 prefill 也不可视为等价工作量。

## 接口与桌面

仅本机管理路由新增 `GET /runtime/performance`，沿用 Host/Origin/Bearer 和控制请求预算；LAN 无此管理路由，但其推理纳入同一 actor。返回严格 DTO `{instance_id, capacity, records}`，capacity 当前为 200，records 最新优先。记录键是本服务真实 proof instance UUID 加单调 sequence；终态后允许重用 request_id，不能单靠它作为历史键。

桌面新增受 ACL 约束的只读 `performance_get`，使用同一 VerifiedConnection，并核对响应 instance UUID；拒绝超界或不一致数据。旧服务 404 显示不支持，查询失败不会使聊天/OCR 失败。独立性能页可筛选模型/状态/类型，查看详情和复制筛选后的 CSV。可见时每 3 秒查询，避免重叠与旧连接结果覆盖；停止/重启不展示为同一服务的历史。

保留现有 SSE、非流式 OpenAI 响应及 ChatEvent 结构。公共 HTTP/proof 协议仍 1，shim 行为身份仍 4、C ABI 布局仍 v2；Completed 私有载荷变化使 worker IPC 从 3 升到 4，父/worker/打包和独立验收器必须同时更新，不允许混用旧 worker。

## 限制与验证

内存历史在服务退出时清空，仅保留终态、无实时逐 token 仪表和断线重放。模型 ID 不是文件 hash，记录不宣称精确复现实验；比较需保持设备、文件、上下文、线程、批次和输入一致，并区分模型冷加载及输出达上限。实际验证见[验证记录](../verification/2026-10-08-inference-performance.md)，不继承旧版本 Windows 验收。
