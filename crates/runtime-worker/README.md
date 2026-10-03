# 独立原生 worker（T03）

`ai-runtime-worker` 是唯一链接 `engine-host` / `llama-adapter` / llama.cpp 的 PC 推理进程。无运行参数、网络端口、隐藏故障开关或第二份 `Runtime`。管理进程及其单一调度器通过 stdin/stdout 的私有 NDJSON 驱动一份 `EngineHost`，该 host 的专用原生线程保持所有 engine/model/context/sampler 的创建与释放归属。

## 接口与边界

- 父进程首先发送 `Hello`：非空 UUID session、operation=0、request/seq=null。worker 读取实际 `llama_adapter::build_info()`，核验私有protocol=2、shim行为身份=3（公共protocol仍1、C ABI布局v2）、llama commit=`2149c00f4442dc59302e134a02e4c99d5f7ed9fc` 后回复
- 之后 `Load` / `Generate` / `Unload` 的 operation ID 非零且严格递增，只有一个执行中的操作。Generate 的 envelope request ID 必须匹配请求内部 ID；Load/Unload 为 null
- `Cancel` / `Credit` 使用对应 operation/request；`Shutdown` 使用 operation=0 且 request/seq=null。未知字段/帧、错误方向、版本/session/请求身份不符、重复操作、未握手命令、超限和截断帧导致安全取消并退出
- Cancel 在控制线程直接设置独立 native 取消标志并唤醒条件变量。它不会排入原生 mailbox，也不会等待生成或 stdout writer
- 信用 ID 全 session 非零严格递增；只允许活动 Generate 有最多两个未消费 credit。每个 text_delta ≤4096 UTF-8 bytes 且消耗恰好一个 credit；worker 从不自动补充 credit
- 终态清空未用 credit。最近完成操作的新 credit 允许迟到但立即作废，绝不供下一请求使用；最近操作的迟到 Cancel 幂等处理，避免终态与在途控制帧竞争。父 supervisor 通过唯一FIFO writer串行命令及credit，观察终态后停止旧授信，再允许下一操作，因此合法在途控制帧不会跨过第二个后续操作。非邻近/重复/乱序 credit 仍拒绝
- 事件 seq 从1开始，整个 session 连续递增。请求最大2MiB、事件最大64KiB，均包含LF；文本编码帧另受协议25KiB限制

## 有界输出与退出

stdout 专用于协议。独立 writer 持有最多一个编码帧，发送队列最多四个帧；生产路径的文本帧还必须持有父进程的一次性预算 reservation。父进程的 reservation 延续到最终消费者释放，worker 不建第二个文本预算、不因管道写成功而退款。信用等待与队列等待都可由 Cancel、EOF、Shutdown 或 writer 失败打断。

正常退出先取消，等待原生终态，再提交内部 Unload，在原生资源释放确认后关闭 EngineHost mailbox。整个清理窗口最多4秒；不会无限 join 被堵塞的 stdin/stdout 或卡住的原生线程。超时退出失败，由操作系统及父进程的五秒 watchdog/进程 containment 最终回收。此机制属于进程隔离，不能用于 Android 强杀线程。

原生 shim 的日志回调不输出正文；Rust 禁止输出 panic payload、路径、参数和生成文本。失败时 stderr 仅一条固定短诊断，不输出解析内容。父进程仍必须限量读取/丢弃 stderr，以覆盖异常二进制或系统诊断。

## 验证

- `cargo test -p runtime-worker`：真实二进制/OS管道握手、EOF/Shutdown、无隐藏argv入口、错误帧/限长/截断，以及信用、状态和可取消等待单元测试
- `cargo test -p runtime-worker --test real_credit -- --ignored --test-threads=1`：设置 `NEXA_TEST_MODEL` 与 `NEXA_TEST_THREADS=2` 后以固定真实模型验证零信用阻塞及旁路取消
- 完整调度与崩溃隔离必须使用 `process-host` 的独立 management harness（其依赖树无 `llama-adapter` / `engine-host`），不能将本 crate 中原生链接的测试进程当成管理进程隔离证据

固定模型、Windows CI 和目标设备的验收范围由项目验证记录单独记载；构建/协议测试不等于真实推理或设备性能结论。

### Linux 开发验证（2026-10-01）

本轮实际运行 `cargo test -p runtime-worker --offline`：10个单元测试、6个真实进程/管道测试通过；真实模型测试为显式 ignored，不混入无模型测试通过数。`cargo clippy -p runtime-worker --all-targets --offline -- -D warnings` 通过。附加的阻塞 writer 测试使用可控阻塞 `Write`，验证 writer 仍被阻塞时控制线程及原生 Unload 已完成；不冒充真实 OS 管道饱和验收。

显式运行 `real_credit` 使用精确模型 SHA256 `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`、CPU 2线程、context2048、batch512、seed42/temperature0。22.54秒完成模型 hash/加载、零信用中文请求取消、单信用英文流159字节、卸载和退出；单次取消至原生失败确认16ms。此数据仅为共享 Linux 开发机单次回归，不是Windows/i5-8400性能承诺。日志位于本地 ignored `artifacts/verification/t03-worker-{tests,clippy,real-credit}-linux.log`。
