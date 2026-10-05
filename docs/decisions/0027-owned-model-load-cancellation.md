# ADR0027：按操作身份手动停止模型加载

日期：2026-10-05。状态：已实施并完成源码与真实 Linux 模型分层验证，原生 Windows/用户目标机待验。

用户要求“增加停止加载功能，有时候会加载很久，我想手动停止”。范围包含模型库显式加载/切换，以及添加或下载完成后可选的加载与短测。停止仅针对本窗口发起的那一次操作，不能退化成停止整个服务或取消别的客户端。

## 身份与接口

前端先用 `crypto.randomUUID()` 生成一次性 `operation_id`，再调用新命令：

- `model_load_start`：原 `LoadModelRequest` 字段加 `operation_id`
- `model_load_profile_start`：原 `ModelLoadProfileRequest` 字段加 `operation_id`
- `model_load_next`、`model_load_cancel`：仅 `{operation_id}`

start 立即返回同一 `{operation_id}`。next 返回 `operation_id/model_id/phase/status/terminal/runtime/local_validation/error`；阶段为 `preparing/loading/testing/finished`，状态为 `running/cancelling/completed/cancelled/failed`。取消回执 `{stopping:true}` 仅表示收到意图；重复取消幂等，终态取消返回 `stopping:false`。错误与不确定通信均不得先宣称资源已经释放。

桥仅保留本窗口当前和上一任务，拒绝未持有的 ID；旧 ID 不定位当前模型。回环管理新增 `POST /runtime/load-operations`、`GET /runtime/load-operations/{id}`、`POST /runtime/load-operations/{id}/cancel`。start 接受已登记模型与原有加载字段，并要求同一随机 UUID；可选 `only_if_unloaded` 仅用于不抢占的自动流程。服务 current/previous 槽有界；保留槽内同 ID/同完整参数幂等，同 ID/不同内容拒绝。不为失效旧 ID 自动重发 start。

新接口仍需要原管理 Bearer、同连接服务端 proof 和回环路由；LAN 路由没有任何加载/取消/状态管理入口。UUID 是持有者控制句柄，不在公共模型列表或全局状态泄露。桌面命令、原生 handler、ACL、能力列表和打包集合一起更新；旧同步 HTTP/bridge 调用保留兼容。

## 取消与清理

1. 文件准备/整文件 hash：操作令牌绑定本次 `ScanControl`。停止后继续等待 blocking 工作和清理完成，期间不释放 registry lease；不删验证、结构/hash/TOCTOU、安全路径门槛，也不伪造原子系统 I/O 可即时中断
2. 模型切换：同一 actor 持有取消令牌贯穿旧模型卸载与新模型加载。切换卸载期间收到停止，等待卸载 ACK，然后跳过新模型，不自动恢复旧模型
3. 原生加载：取消直接进入独立控制标志，不排在 Load 命令后面。独占显式加载期间拒绝其他生成入队，避免取消别人的请求或取消后被队列自动重载
4. 取消与 Loaded 交叉：若已看到取消，补做卸载并等原生资源清理 ACK 后才结束。ProcessHost 对不合作 worker 沿用5秒取消宽限、终止与有界回收；`CleanupUnconfirmed` 永久故障优先，坏IPC/原生Faulted等真实故障也不能被Stop意图覆盖为正常取消。EngineHost 失败/取消 Load 也必须先释放 engine 再发 ACK
5. 首个原因：在加载 deadline 前收到的手动停止，不因清理耗时跨过 deadline 被改成 load_timeout；已成立的超时/真实故障继续保留
6. 已加载后的短测：只取消该操作创建的私有 probe RequestId，等其终态。此时模型可以继续驻留，且可有其他客户端随后使用；不能为了停止测试卸载或抢占它们。取消不落成 Failed 模型证据，历史证明不冒充本次通过

prepared external 文件保护的生命周期不缩短：Unload 本来就不等于释放所有源文件 guard；仍按原有服务/worker 关停边界释放。停止操作不会修改服务关闭偏好或实例全局状态。

## 通信故障、终态与自动流程

开始时 ID 已在前端与 bridge 固定，因此任一层 start 回执丢失均可用原 ID 查询。bridge 只发送一次 POST，回复挂住期间仍并发查询/取消；错误 ID、非法观察或连接错误不释放 busy，显示结果待确认。所有请求绑定最初同连接证明的 service instance；看到新实例即终结旧任务为 `runtime_shutdown`，不重发到新服务。已确认受理的操作返回同实例404，按“仅终态可被槽淘汰”报告结果过期，不能把它当成功或取消。完全不确定的 start 保守保留占用，直至拿到确切结果或确认原服务已经结束。

`completed` 表示流程返回，可包含 Passed/Loaded/Deferred/Failed/Stale 等真实短测观察；不能统一显示验证通过。`runtime` 可为空，表示最后状态观察缺失，需要重新检查。窗口关闭取消本窗口任务并有界等待；无法确认时关闭报错可重试，不代为停止全服务。

添加/下载自己的取消按钮沿用原任务身份。登记/下载结果已经发布时停止后续加载或短测，保留“已保存/已登记”事实；内部调用复用 tracker，不再取得同一 work 锁。其 DTO 的可选 `load_phase` 来自该任务真实操作，界面在准备/加载阶段写“停止加载”，短测阶段写“停止本次测试”。自动 actor 准入仍原子要求未选择/未加载且空闲，绝不抢占别人的已驻留模型。

验证与未覆盖条件见[本轮记录](../verification/2026-10-05-model-load-cancellation.md)。
