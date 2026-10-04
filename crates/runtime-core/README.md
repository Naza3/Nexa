# runtime-core

T02 的单一调度 actor，不链接 llama.cpp，也不依赖 HTTP、Tokio 或 Flutter 类型。`Runtime::spawn(config, resolver, executor)` 启动一个控制线程；`RuntimeHandle` 提供同步的 submit/load/unload/cancel/status/shutdown。T04 Tokio API在有界blocking边界调用，控制与存储容量独立，不能在异步reactor等待同步load。T03子进程执行器由独立process-host组装。

## 不变量

- 只有 actor 修改 selected_model、状态、活动槽和 FIFO。默认一个活动请求 + 八个等待请求；Android配置一个等待请求
- 首次有效请求选择模型；其他模型返回 ModelConflict，不自动置换。显式切换只在没有请求时按 Unload ACK → Load 串行执行
- 每次真正Load（含idle重载和显式恢复Unload后）重新调用ModelResolver，不能把selected里的旧loadable/path当永久许可。模型指纹/候选资格/ID/context变化时不向native投递Load，保留选择和原context参数，外部重新验证后由显式Load恢复
- ModelResolver 只做有界元数据/路径查找。导入、完整SHA-256校验和缓存刷新在启动前或actor原子RegistryLease预约下的blocking任务执行；无合法loadable或超过模型metadata与131072硬限的请求不能加载；validated仅保存历史实测证据，不是型号/hash白名单，候选最终仍须原生loader与模板接受
- Executor::start 必须快速返回，调用方不能在其中同步推理。只有独立 CancellationHandle 可跨线程；所有原生创建和释放归执行器线程
- queue/load/execution 分开计时，execution 包括 prepare/prefill/decode。取消和超时只设置控制意图；活动生成槽一直保留到 executor 清理后的终态 ACK
- 公共事件有逐请求递增 seq，且只有一个终态。精确 prepare 通过后发布 Started；消费者断开后仍在内部终结，但不强行交付
- 一个共享预算覆盖 executor→actor→消费者全部在途文本，最多 256 KiB；delta 按 UTF-8 边界切到最多 4 KiB。短 delta 至少计费 256 字节，避免海量小事件绕过内存界限；最多1024个文本槽
- 预算满时执行器回调可有界等待；十秒没有消费进展便取消为 SlowConsumer。用户取消、断开及关停唤醒预算等待；终态不会再等待文本空间
- 队列最多8项、控制mailbox64项、执行器event mailbox32项，均有界。慢消费者检测在等待共享预算时进行，不能无限等待写终态
- idle 只在无活动/队列且 Ready 时计时；卸载保留选择和参数。边界到达的新请求安全等待卸载完成，然后同模型重新加载
- Faulted 批量失败且不重放；显式 load 会先确保旧资源卸载。错误顺序事件会取消执行器并等待真实终态 ACK，不能提前复用原生槽
- Runtime::shutdown等待安全释放后调用Executor::close，传播进程回收失败。Drop只请求安全关停，不强杀卡住的原生线程；process-host可按五秒取消界限回收整个worker
- 父端专用CleanupUnconfirmed使所有受影响请求Failed，覆盖普通取消但保留原因；永久禁止Load，status保留Faulted，shutdown返回明确错误。该事件不表示资源已回收，也不允许worker发送
- ExecutionEvents::from_sink让worker直连EngineHost；不创建第二个Runtime。TextPermit绑定原始预算和operation且不可Clone；丢弃只退自己的账
- recv_leased/recv_timeout_leased返回EventLease，移出队列不算消费，写完后drop才退账。兼容recv/recv_timeout把返回视为消费；HTTP/SSE必须保留lease到实际write完成
- Output与预算分别持有，permit只持预算，避免队列中的permit形成Arc环。断开只清队列，外部lease与在途permit继续持账直到各自释放

GenerationRequest 的 options 全部显式传入；`GenerationOptions::default()` 仍是通用512 token。`RuntimeConfig::android()` 覆盖调度/加载参数，移动桥在T07应用未提供请求预算时的256默认值；当前没有移动HTTP/UI默认值实现。

T04注册预约：`RuntimeHandle::reserve_registry`只在actor判定无活动/队列/操作/关停时授予排他lease；持有期间拒绝submit/load/unload并推迟idle unload，status/cancel保持响应。lease Drop非阻塞，shutdown通知取消且等待真实存储清理完成。Faulted下导入不会改变执行器故障或恢复它。

T04输出适配：`DisconnectHandle`允许HTTP在receiver被移动到blocking事件泵后仍非阻塞取消；`EventLease::retain_permit`销毁事件后仅收缩旧预算，不创建新账本或伪报socket消费。core消费进展仍以完整delta/lease释放为粒度。

## 测试

```sh
cargo test --locked -p runtime-core -p runtime-types
cargo clippy --locked -p runtime-core -p runtime-types --all-targets -- -D warnings
```

`tests/scheduler.rs` 用受控执行器验证A05–A12的调度逻辑，绝不算作模型推理或设备验收。真实 model-store→runtime-core→engine-host→llama.cpp 链路在 engine-host 的 `real_runtime` 可选测试中执行。
