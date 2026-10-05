# ADR 0004：T03进程隔离与单一输出账本

日期：2026-10-01。状态：已实施，固定Windows CPU阶段门槛已通过；精确提交与实际验证范围见[T03记录](../verification/2026-10-01-t03-worker.md)。未授权合并、部署或扩展产品范围。

## 决策

保留T02的EngineHost作为原生专用线程执行器，新增纯Rust process-host与runtime-ipc。父管理进程只组装一个Runtime，worker只通过ExecutionEvents::from_sink直接调用EngineHost。避免以feature命名掩盖native依赖，也避免worker再建一个调度器导致双重取消、预算及状态。

父子先Hello，身份为协议1/shim2/llama commit `2149c00f4442dc59302e134a02e4c99d5f7ed9fc`；worker身份来自实际build_info。session、operation和request是三个不同范围，worker seq不等于公共请求seq。详见[规格6.4](../../ai-runtime-v0.1-execution-spec.md#64-pc-ipc)及[协议源码](../../crates/runtime-ipc/src/lib.rs)。Shutdown是operation=0的session控制；取消与信用绑定具体操作。最近终态的迟到新信用被退休，不能重新用于下一操作。

## 预算与消费

旧方案把出队视为消费。进程IPC/SSE还会持有出队后的编码或写入，提前退款将允许多层各堆一份。新TextPermit不可复制且绑定原账本与operation；队列、transport、actor、EventLease移动同一permit。Output与Budget分离，不形成Output→permit→Output强引用环。失败、旧操作、取消、断开、写失败、worker死亡和未用信用均通过各自permit析构归还，禁止把总数直接清零。

PC第一版每请求固定预留16 KiB暂存，最多两个120 KiB信用，总计256 KiB。每信用只准一条≤4 KiB UTF-8片段，完整编码≤25 KiB。120 KiB保守覆盖该片段在worker编码（Vec容量≤32 KiB）、pipe（≤25 KiB）、父端输入（Vec容量≤32 KiB）、解码/serde/DTO及移动过程中的文本表示；其余固定读缓冲/回调暂存计入16 KiB。双方只用一个读/写序列，不保留额外文本历史，worker不自行补信用。实施时的额外clone须逐项复核，不能把两个有界队列冒充一个预算。

这是一份合规待发送输出的字节账本，不是整个进程堆的物理峰值保证。模型、KV、native tokenizer/stop缓冲（包括上游大token piece）、请求输入、独立受限的畸形帧解析、结构/分配器开销与线程栈另计。必须分项报告，不能宣传整个runtime总内存≤256 KiB。

EventReceiver的兼容recv把返回视作消费；新的recv_leased使charge保持到实际消费/丢弃。T04写SSE必须持有lease到write完成。慢消费者时间从真正信用不足开始，只有消费/丢弃更新进度，长prefill、发信用和搬运pipe不延长消费期限。默认十秒后核心发出SlowConsumer取消；五秒worker取消宽限仍单独计时。

代价是同时最多两个未消费delta，短token密集输出可能被IPC往返及core/进程supervisor控制轮询限制。尚无证据保证吞吐额外损耗≤10%；必须与相同模型/线程/采样的最小原生链路实际比较。后续扩大窗口需重新证明全链路账本，不能直接放宽为两边各256 KiB。

## 清理、故障与恢复

start快速返回。父supervisor负责生命周期；读、写与取消意图独立。首次取消起五秒内无清理ACK则终止整个worker，确认已退出并回收后才送Faulted。生成正常错误只在安全清理后回Ready。Ready闲置死亡也报告Faulted。当前取消保留取消原因，队列统一终结，不自动重放部分输出。

显式Load是Faulted后唯一重启入口。核心已有Unload→Load恢复链；已死且回收完毕的worker可对恢复链内部ExecutorCommand::Unload立即确认，不能无意重启；Faulted下公共RuntimeHandle::unload仍拒绝，保持规格6.1。Runtime shutdown在资源ACK之后调用Executor::close并传播回收错误；不能把Drop当成已经reap的证据。

极少数OS kill/wait/job错误可能使死亡无法确认。父端专用CleanupUnconfirmed不冒充安全Faulted ACK：核心将current/queue全部Failed(executor_cleanup_unconfirmed)，覆盖先前取消但在无敏感诊断中保留原原因；保留Faulted可查询，永久禁止此执行器再次Load。shutdown有界返回错误，ProcessHost保留未确认PID、不增加reaped，清理OS句柄仅为尽力兜底。该variant不能serde到wire，伪造相同错误码也拒绝；不能用worker自报解除containment约束。

Windows目标要求无继承的Job handle + KILL_ON_JOB_CLOSE及创建时原子Job归属；不可用时失败关闭，不退化成无containment执行。Linux只是开发探针，采用父死亡信号/父PID复核和独立进程组回收。实际实现和平台结果必须由测试记录证明。

## 模型重载身份

selected仅记忆模型ID和加载参数。每次actual Load都重新调用有界ModelResolver，包含idle卸载后的自动重载及显式Load恢复链中的Unload之后；重新核对ID、validated与context限制。元数据/指纹变化或未验证替换会在native Load前失败，不能沿用旧path/validated许可；外部完整校验通过后才允许显式恢复，不在actor做大文件hash、不静默降低context。

## 验证边界

codec与permit逻辑测试不是模型推理。无native依赖harness证明父管理进程可独立构建；T03尚无HTTP，不能称HTTP/API存活验收已完成。真实GGUF、异常各阶段、取消强杀、正常和异常父退出、Windows Job与目标设备分别记录，不互相替代。T04、T05独立Windows无开发工具验收机仍独立待办。
