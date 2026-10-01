# ADR 0003：T02 调度、存储与原生可观测边界

日期：2026-10-01。状态：实现决策；目标平台验收以对应报告为准。

## 背景

T00/T01 已在固定 Windows x64 CPU / 2线程 / Qwen3-0.6B Q8_0 / context2048 组合通过。T02 需要真实模型管理和调度，又不能提前声称 T03 进程隔离或 Android 真机支持。

## 决策

1. `runtime-core` 使用一个 Rust 专用控制线程持有全部状态，以有界 mailbox 串行控制；原生资源完全留在 `engine-host` 的另一个专用线程。core 仅依赖 runtime-types，未来 Tokio/Axum 接入跨 blocking 边界调用，不让 HTTP 管理进程链接原生库
2. `ModelResolver` 为有界元数据/路径查找；model-store 在 open/import/显式verify 时校验完整文件并缓存指纹。resolve发现文件或manifest变化即拒绝并要求重验，不能在 actor 内悄悄重新hash数百MiB文件
3. 存储的原子提交单位是包含 model.gguf 和 manifest.json 的整个模型目录，索引由已提交manifest派生。不使用两个独立rename假装双文件事务。失败清理staging，store进程锁只协调遵约客户端；应用私有目录不是同UID恶意写入的安全隔离边界
4. 公共 ModelId 保持 `[a-z0-9][a-z0-9._-]{0,63}`。可移植文件存储额外拒绝 Windows 设备保留名及尾点，防止同一ID在两平台指向不同对象；这属于导入的可用性检查，不能悄悄改共享ID语法
5. 一个共享文本预算贯穿原生回调、内部event和消费队列，最多256KiB；UTF-8 delta最多4KiB，小delta按最少256字节计费以约束事件开销。同步回调允许在decode步骤之间作可取消的有界背压等待，不允许无限阻塞
6. prepare通过后才能Started；取消/超时保持原生槽直到清理ACK。超时取消只属于协作式线程控制，PC五秒强杀必须在T03实现；Android不杀原生线程。Faulted仅显式load恢复，不重放部分输出
7. shim build_info版本升为2，新增 `air_generate_observed` 并保留 `air_generate` 兼容包装。进度只包含PrefillStarted、PrefillBatchCompleted和DecodeStarted的数值，不包含正文。中途prefill验收在至少一批成功、仍有后续prompt的观察点跨线程取消，不能拿预取消或首token后取消代替
8. RequestTimings公开queue/load/execution总时长；host尽力交付的16条有界数值诊断另报load/prepare/prefill/decode，不构成新的可靠公共事件流。GenerationFailed和公共失败/取消保留实际usage
9. 当前GenerationRequest必须显式给完整options。移动桥T07负责应用Android缺省256输出预算；通用GenerationOptions默认512不随RuntimeConfig自动改写，core不悄悄改变调用者预算

## 验证与限制

调度fake仅用于有界性、竞态、deadline及状态回归。真实GGUF链路和独立prefill/decode取消有Linux开发证据；Windows新增工作流必须实际通过后才记录T02目标平台门槛。T03崩溃隔离、T05独立验收机、Android真机、长时内存趋势和摘要业务质量均未完成。

入口：[core](../../crates/runtime-core/README.md)、[host](../../crates/engine-host/README.md)、[本轮验证](../verification/2026-10-01-t02-runtime.md)。
