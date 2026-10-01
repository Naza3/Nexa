# runtime-core

T02 的单一调度 actor，不链接 llama.cpp，也不依赖 HTTP、Tokio 或 Flutter 类型。`Runtime::spawn(config, resolver, executor)` 启动一个控制线程；`RuntimeHandle` 提供同步的 submit/load/unload/cancel/status/shutdown。将来 Tokio API 应在 blocking 边界调用，不能在异步 reactor 上等待同步 load。暂未实现 T03 子进程或 T04 HTTP。

## 不变量

- 只有 actor 修改 selected_model、状态、活动槽和 FIFO。默认一个活动请求 + 八个等待请求；Android配置一个等待请求
- 首次有效请求选择模型；其他模型返回 ModelConflict，不自动置换。显式切换只在没有请求时按 Unload ACK → Load 串行执行
- ModelResolver 只做有界元数据/路径查找。导入、完整 SHA-256 校验和缓存刷新在进入 actor 前执行；未知或超过已验证上下文的模型不能加载
- Executor::start 必须快速返回，调用方不能在其中同步推理。只有独立 CancellationHandle 可跨线程；所有原生创建和释放归执行器线程
- queue/load/execution 分开计时，execution 包括 prepare/prefill/decode。取消和超时只设置控制意图；活动生成槽一直保留到 executor 清理后的终态 ACK
- 公共事件有逐请求递增 seq，且只有一个终态。精确 prepare 通过后发布 Started；消费者断开后仍在内部终结，但不强行交付
- 一个共享预算覆盖 executor→actor→消费者全部在途文本，最多 256 KiB；delta 按 UTF-8 边界切到最多 4 KiB。短 delta 至少计费 256 字节，避免海量小事件绕过内存界限；最多1024个文本槽
- 预算满时执行器回调可有界等待；十秒没有消费进展便取消为 SlowConsumer。用户取消、断开及关停唤醒预算等待；终态不会再等待文本空间
- 队列最多8项、控制mailbox64项、执行器event mailbox32项，均有界。慢消费者检测在等待共享预算时进行，不能无限等待写终态
- idle 只在无活动/队列且 Ready 时计时；卸载保留选择和参数。边界到达的新请求安全等待卸载完成，然后同模型重新加载
- Faulted 批量失败且不重放；显式 load 会先确保旧资源卸载。错误顺序事件会取消执行器并等待真实终态 ACK，不能提前复用原生槽
- Runtime::shutdown 等待安全释放。Drop 只请求安全关停，不强杀卡住的原生线程。Windows五秒强杀恢复属于T03，不把当前进程内线程实现当作Windows产品架构

GenerationRequest 的 options 全部显式传入；`GenerationOptions::default()` 仍是通用512 token。`RuntimeConfig::android()` 覆盖调度/加载参数，移动桥在T07应用未提供请求预算时的256默认值；当前没有移动HTTP/UI默认值实现。

## 测试

```sh
cargo test --locked -p runtime-core -p runtime-types
cargo clippy --locked -p runtime-core -p runtime-types --all-targets -- -D warnings
```

`tests/scheduler.rs` 用受控执行器验证A05–A12的调度逻辑，绝不算作模型推理或设备验收。真实 model-store→runtime-core→engine-host→llama.cpp 链路在 engine-host 的 `real_runtime` 可选测试中执行。
