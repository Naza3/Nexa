# ADR 0005：T04 本机 HTTP、受控管理与输出预算

- 日期：2026-10-01
- 状态：已实现，Linux验证收口；Windows实际验收单列，本文不替代真实模型/Windows证据
- 范围：runtime-api、runtime-cli、runtime-core 的注册预约与输出 lease；无合并、部署、系统服务或用户真实持久凭据初始化

## 1. 契约与界限

沿用执行规格第 7/8 节的十条基础路由及 Chat Completions 文本子集，新增鉴权 `GET /runtime/models` 提供包含未验证模型的安全管理摘要。禁止通过 API 提交完整 manifest、source、relative_file、validated 或任意 extra，导入只接受显式绝对本地普通文件路径；不下载模型。chat/load 的 model 只接受 ModelId。

`/runtime/models` 与 `/v1/models` 都采用可选 `limit`/`after`：默认 64、最大 128，after 与 next_after 为 ModelId，按 ID 升序游标翻页，末页 next_after=null。调用方需遍历分页。前者只输出 ID、显示名、大小/hash、架构/量化、验证/可用状态和已验证 context 摘要；后者只输出可用模型的标准 list/data/model/id/owned_by 结构。现有 manifest 没有可靠创建时间，因此不虚构 created=0；分页和缺省 created 是明确的首版兼容边界，不能称完整 OpenAI API。

未知字段指出字段名；JSON 各层重复字段拒绝。tools、multimodal content、response_format 等不能静默忽略。仅规格列明的零 penalty、logprobs=false、tool_choice="none" 等是空操作。应用层HTTP拒绝使用统一ApiError包裹，内部路径、正文与凭据不进入公开诊断。Hyper进入middleware前的畸形HTTP framing/header可能直接400空正文或关闭连接，不承诺这些传输协议错误也带JSON包裹。生成阶段 NativeFailure 不误报为 model_load_failed。

## 2. 原子注册预约与 blocking 边界

RuntimeHandle::reserve_registry 向唯一 actor 请求 RegistryLease。actor 在同一状态机中检查活动任务、队列、load/unload、关停及其他预约；API 锁或先查 status 再导入不能代替此判定。取得预约后才在 actor 外执行复制、hash 与注册；同步任务持有 lease 直到成功提交或失败 partial 清理实际结束。

lease Drop 只写原子释放信号，不阻塞向可能已满的 mailbox 发送。授予后调用方消失同样自动释放。持有预约时拒绝 submit/load/unload，并延后 idle unload。status/cancel 保持可调用；shutdown 请求取消且等待 lease 清理完成。Faulted 包括 CleanupUnconfirmed 下允许纯存储导入，但不清除故障、不改变模型选择、不创建或重启 worker；不可恢复的执行器依然不可恢复。

Runtime 同步 ask 全部跨有界 spawn_blocking：普通请求/管理 32 槽、控制查询/取消独立 8 槽、存储 1 槽，容量耗尽立即报忙，不能无界排入 Tokio blocking 队列。服务启动时 Store open 的全量 hash 也在 blocking 边界完成。模型摘要在启动和成功导入后更新，查询锁内只截取一页，不等待导入锁或复制完整注册表。

每次导入使用独立取消 token。HTTP future 断开/drop 取消该次导入，关停 watcher 也取消它；已完成操作不留 watcher，后续导入不受旧 token 污染。HTTP返回失联不撤销已实际完成的原子提交。若导入在rename提交后发生fsync/最终确认失败，必须先以精确ModelNotFound确认该ID原不存在，再在仍持有lease时bounded get核对新注册项并upsert摘要；仅确认cached resolve可执行时才标available。仍返回500 `import_committed_durability_unconfirmed`，说明已注册但最终确认失败、请查列表，不自动重试/删除或谎称成功。不能确认提交时保留原安全错误；不按原始Io消息字符串猜测。Store注入commit后同步失败测试与API纯结果分类测试分层证明这一点，不增加生产HTTP故障hook。

普通文件检查既在打开前也在打开后。Unix 使用 O_NOFOLLOW|O_NONBLOCK，避免路径竞态替换成 symlink/FIFO；Windows在打开前拒绝DOS设备/网络namespace，使用OPEN_REPARSE_POINT，打开后要求GetFileType==FILE_TYPE_DISK且拒绝reparse属性，不读取设备。源文件不删除或修改。

## 3. 配置和事实报告

规格默认 context_size=4096、batch_size=512、max_output_tokens=512 不变。当前模型的注册解析执行上限为context2048，较小逻辑context继续受core和adapter的预算检查；不在API硬编码某个模型或精确context。默认4096超过该模型上限时返回明确context_length_exceeded，不静默降级。真实 smoke 显式 context2048/threads2/batch128/backend cpu。

API 装配层未提供 threads 时取 min(4, available_parallelism)，查询失败回退 1；显式用户值不修改。报告线程来源、可观测可用并行度及超配，不把测试过的两线程证据泛化为任意线程数。模型状态来自 core，worker PID/session/reaped 来自 ProcessDiagnostics。尚无可靠实测的内存、GPU 与 native backend/device 探测用 null/unavailable，不把配置值或构建能力伪装为原生探测。

## 4. 本机安全与服务端身份

只监听回环地址；所有真实连接端点由 accept loop 从 socket 注入。精确数字 Host 必须匹配实际监听 socket，默认拒绝任何 Origin；配置的可信 Origin 也必须精确匹配。除 healthz 外全部 Bearer 鉴权，正文在进入 handler 前有 1 MiB 硬上限（配置可更低），包括 chunked/未知路由。无 CORS 通配符、公开监听、代理 header 身份或请求正文日志。

healthz 正文仍只表存活。可选 HMAC-SHA256 challenge/proof headers 让 CLI 在传 Bearer 前认证服务端；32 字节安全随机 nonce、域分隔版本、实际 instance UUID 和同一 socket 的 client/server 端点都进入 MAC。CLI以标准恒时MAC验证成功后，先等待该固定HTTP/1 sender可发送状态，再上传Bearer；ready等待与一次send共用原请求总deadline，不重试、不重连、不重定向。proof本身仍在原5秒总时限内。公开 instance ID 只负责发现关联，不是认证。端点绑定防止把健康 proof 从另一连接中继过来。此机制不抵御已经可以读取该用户 secret 的同用户/管理员对手。

init 才能创建凭据；serve 不自动 init。凭据从创建时即当前用户私有，拒绝 symlink/reparse、非私有权限、错误 owner 和非普通文件。测试仅使用临时数据目录与临时 token。

## 5. HTTP 输出与关停

早期413等拒绝必须明确Connection:close。已按RFC9112§9.6处理HTTP错误回复与仍在上传客户端的关闭竞态：Hyper成功poll_without_shutdown、写队列flush后才写半关闭（FIN），再有界丢弃输入。固定8KiB scratch、总1秒、总1MiB+64KiB（包含已有Hyper read buffer），不可被持续来字节延长；不解析或派发pipeline，不计入生成文本256KiB账本。错误或超界仍终止连接，不为任意大/无限上传无界排空。headers-only和暂停客户端读取50ms的小发送缓冲eager上传分别测试完整413；原eager broken-pipe红测及同一断言绿测保留。依据：[RFC9112 §9.6](https://www.rfc-editor.org/rfc/rfc9112.html#section-9.6)。

SSE 在 Started 后才返回正常 200；排队、加载、模板和精确预算失败仍使用对应 HTTP 状态。成功为 role→文本→finish→可选 usage→[DONE]；开始后错误只发 error 并关闭，绝不发成功尾部。EventLease 必须留到 Hyper 实际提交对应 socket 写入数据，而非读出 core 事件时退账。Hyper 固定1.11.1，writev=true、pipeline_flush=false；源码src/proto/h1/io.rs的Queue路径保留原Buf，chunks_vectored写出后仅按实际字节advance，src/common/buf.rs只在完整消费后pop/drop。bytes1.12.1的Bytes::from_owner固定owner并取原slice，无deep copy；不能转BytesMut或启用Hyper Flatten。已按这些源码核验，内存writer部分写/Pending及真实socket边界另有契约测试。

core消费进展维持完整delta/EventLease释放粒度；socket实际正数写入只重置连接write-stall计时。一个delta持续超过10秒未完整消费时，core仍可保守SlowConsumer取消；没有新增裸note_progress，不宣称core逐字节推进计时。

非流式完整 JSON（含封套、转义、usage）上限 96 KiB。第一笔 120 KiB IPC permit 通过只缩小的转移保留 96 KiB，后续事件按同一账本消费；保守稳定峰值为 96+120+16=232 KiB，仍在 256 KiB 账本内。编码超限立即取消，返回 400 / invalid_request_error / response_too_large / param=stream，建议流式或减小输出；不截断、不落盘、不重放。不能把此账本宣传为整个进程/模型/输入/分配器内存限制。

HTTP shutdown、Ctrl+C、bind/启动失败和传输异常统一 ServiceShutdown：停止接受新任务、取消导入和生成、等待 core 及 worker 真实清理。shutdown 仅清理确认后返回 200；CleanupUnconfirmed 保持失败并使 CLI 非零，不能把请求已发出或连接消失称为已停止。CLI stop 验证原实例服务端 proof，等待成功响应和该实例锁释放/记录清理；不猜测或强杀 PID。

## 验证职责

协议 fake 仅验证边界和调度，不能替代真实 GGUF。Linux 开发验证、真实 HTTP/CLI smoke、无 native 管理端构建、Windows CPU CI、用户目标机与发行独立机分别记录。本文不声称尚未完成的 Windows/T05 验收。
