# engine-host

T02 的真实原生执行器。依赖方向为 engine-host → runtime-core/runtime-types/llama-adapter，runtime-core 不依赖或链接原生库。当前只有进程内专用线程；Windows 独立 worker/强制终止属于 T03。

- `EngineHost::new()` 创建 `nexa-inference` 线程；`Executor::start` 只放入一个有界命令并立即返回独立取消句柄
- Load 在推理线程创建 Engine、Model/context；Generate 在相同线程创建/消费 Prepared/sampler；Unload 在 Model 和 Engine 均释放后才确认
- 原生句柄保持 `!Send/!Sync`，只有独立 `CancelHandle` 可跨线程设置；取消不会排在生成队列后方
- native prepare 通过完整模板/tokenizer 预算后先发 Prepared；同一有界 FIFO 将公共 Started 排在所有 TextDelta 前，预算失败不发 Started
- 文本回调借用 UTF-8，直接进入 core 的共享 256 KiB 预算，不另建副本队列；等待可取消并受 slow-consumer 时限约束
- 原生返回及清理后发送一次 Completed/GenerationFailed，保留实际 usage；panic 先在 owner 线程清理资源，再报告一次 Faulted
- Drop 请求取消并关闭 mailbox，不强杀原生线程，也不会在其他线程释放 native；需要可确认的资源回收时使用 Runtime shutdown/unload
- `take_diagnostics()` 可取得16条有界、尽力交付的数值报告，包括 load / prepare / prefill / decode 独立时长。取消时当前阶段包含直到 native 返回的清理耗时；诊断不记录输入、输出或路径，不替代公共终态

## 真实集成验证

先构建最新 shim，设置 `AIR_NATIVE_DIR` 后运行：

```sh
NEXA_TEST_MODEL=/path/to/Qwen3-0.6B-Q8_0.gguf NEXA_TEST_THREADS=2 \
  cargo test --locked -p engine-host --test real_runtime -- --ignored --test-threads=1 --nocapture
```

测试将原始模型复制到独立临时 model-store，核验固定文件/模板身份，真实执行预算拒绝、Started顺序、有限FIFO、队列取消、decode取消、同模型恢复、空闲卸载/重新加载及安全关停。原始文件不修改。该用例刻意限定2推理线程/context2048；4线程超配不属于通过配置。A08的独立mid-prefill证据在 llama-adapter 的真实测试，不用模拟执行或时间猜测替代。
