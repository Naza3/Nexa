# ProcessHost：父进程执行器

本 crate、`nexa-process-harness` 和独立故障工具 `nexa-fault-worker` 都不依赖 `engine-host` / `llama-adapter`，管理进程不链接 llama。真实原生执行只在单独的 `ai-runtime-worker` 二进制中。调度器仍仅有父进程 `runtime-core` 一份。

## 生命周期与有界性

- `Executor::start` 只向容量 1 的 supervisor mailbox 投递；取消句柄只记录第一次取消的原子时间戳，不操作管道
- supervisor 生命周期线程负责 spawn / 状态 / 5 秒取消宽限 / kill / reap；独立 stdin writer 和 stdout reader 负责实际阻塞 I/O
- writer mailbox 容量 4；reader mailbox 容量 1，入队前验证 Hello、session、operation、request、seq、阶段、终态及一次性 credit；请求先完整编码和检查大小，才写第一个字节
- Start、Credit、Cancel 共用单 writer FIFO；旧 operation 的已排队控制消息一定先于下一 Start，不会让取消提前到达未定义操作
- Hello 校验 protocol 1 / shim 2 / 固定 llama commit；新进程有新 session，不自动重放部分输出
- 每次 Generate 先向 core 唯一文本账本保留 16 KiB scratch，再最多发出两个每个 120 KiB 的 credit。收到文本时把 permit 移交 core，而不退款；只有消费者完成使用并 drop lease 才可恢复预算。未使用 credit / scratch 在终态回收
- 尚未投递 writer 的取消在本地发普通 `Failed(RequestCancelled)`，不发送未知 operation 的 Cancel、不让健康进程进入 Faulted。已投递操作取消从第一次原子时间戳起计算 5 秒；重复取消不延长宽限
- EOF、协议错误、意外退出、worker 自报 Faulted 都先终止整个容器并确认退出，然后向 core 发一次 Faulted。Ready 闲置死亡也报告；普通 GenerationFailed 保留进程可用
- 死后内部 `ExecutorCommand::Unload` 只 ACK 已回收状态，不启动进程；只有 Load 创建新 worker。公开 Faulted API 仍只允许查询、关闭、显式 Load，不能把内部 Unload 行为解释成扩展公开协议
- Hello / Unload / Shutdown 默认各 5 秒，可配置 1 ns–300 秒；OS kill 后确认退出最多另等 5 秒，I/O 线程退出再最多等 5 秒。极端清理错误进入永久 `CleanupUnconfirmed`，不清 PID、不增加 reaped、不创建新 session，close 返回明确错误；不冒充安全资源 ACK

## OS 容器

Windows 10 / Server 2016 起使用 `windows-sys 0.61.2` 的 SDK 定义：非继承 Job 设置 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`，通过 `CreateProcessW` 的 `STARTUPINFOEXW` + `PROC_THREAD_ATTRIBUTE_JOB_LIST` 原子关联 worker。另用 `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` 精确继承 stdin/stdout/stderr，Job 本身不继承。缺少相关 API 能力或 Job 关联失败时拒绝启动，不降级成 spawn 后 assignment。可执行路径独立传 applicationName，参数按 MS CRT 规则转义，拒绝 NUL，所有 owned handle 由 RAII 释放。

API 依据：[进程属性及 Job/Handle 列表](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)、[CreateProcessW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)、[Job 扩展限制结构](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_extended_limit_information)。

Linux 仅为开发 fallback：子进程 pre_exec 设置独立 process group、`PR_SET_PDEATHSIG(SIGKILL)`，再核对 parent PID；创建线程保持到 reap。正常回收 killpg 后有界 wait，父进程异常退出时保证 worker 自身停止。Linux **不承诺异常父退出后任意孙进程清理**；该项只在 Windows Job 路径测试，不借 Linux 开发结果扩展产品支持矩阵。

## 验证入口

```sh
cargo test -p process-host --locked
cargo clippy -p process-host --all-targets --locked -- -D warnings
cargo check -p process-host --all-targets --target x86_64-pc-windows-msvc --locked
cargo tree -p process-host --locked
```

`process_contract` 用单独 fixture 覆盖错误/缺失/畸形/超限 Hello、load/generate/unload 崩溃、Ready 闲置死亡、session/op/credit 错乱、活着却报告 Faulted 的 worker、stdin/stdout 阻塞、5 秒强杀、重复取消、真正消费者 lease 持有、慢消费者原始原因、队列一次终态、不重放、显式恢复、编码请求越界，以及死后内部 Unload。

`containment` 启动真正独立的管理父进程，检查正常退出 worker+后代回收、异常退出、启动阶段父退出、空格/Unicode 路径。Windows 分支另检查异常父退出后的任意后代；Linux 不运行该额外断言。

固定真实模型管理链路（所有原生代码在外部 worker 中）：

```sh
nexa-process-harness real /absolute/path/ai-runtime-worker /absolute/path/model.gguf
```

固定 context 2048、2 CPU 线程、batch 128；检查中文文本/Usage、卸载再加载后的英文恢复、显式取消、shutdown 后 spawn==reap。只输出数值 JSON，不输出模型路径或生成正文。它是 T03 开发/CI 验证工具，不是 T04 产品 CLI，不验证模型导入来源；必须给已在模型矩阵锁定并核对的本地 GGUF。

Windows 交叉 `check` 只能确认类型和编译，不证明 Job 运行行为；必须再执行实际 Windows CI。Linux 与 CI 都不替代目标 i5-8400 Windows 实机验收。
