# llama-adapter

T01 的 Rust 安全包装，原生实现固定为 `native/llama-shim/include/air_llama.h` ABI v1。

- `Engine::load(&mut self)` 返回借用 engine 的 `Model`
- `Model::prepare(&mut self)` 应用真实模板、tokenizer 和上下文预算，返回借用 model 的 `Prepared`
- `Prepared::generate(self)` 每条请求只消费一次，返回正常结束或带真实 usage 的错误；丢弃未运行的 Prepared 也会清理
- engine/model/prepared 不能跨线程；`CancelHandle` 是单次、可克隆、可跨线程的独立原子取消标志
- 回调借用完整 UTF-8，最多 4096 字节。调用方必须快速复制/有界入队，不在回调内阻塞。回调 panic 在 C ABI 内被捕获，原生清理后在 Rust 栈恢复（`panic=unwind`）
- 原生错误仅供受信任宿主诊断，不能未经清洗通过 HTTP 暴露。示例只记录数值、hash 和错误类别，不记录消息内容
- seed `u32::MAX` 是上游随机哨兵，其他 seed 只在同模型/后端/硬件/工具链内尽力复现

## 构建

默认 build.rs 在 Cargo OUT_DIR 中通过 CMake 构建锁定的 CPU 静态库。也可设置 `AIR_NATIVE_DIR` 指向已完成的独立 CMake 构建目录。该目录须只包含一套静态库，不能混合 Debug/Release 或架构；更新 C++ 源码后必须先重建该外部目录。

```sh
cmake -S native/llama-shim -B build/native-release -DCMAKE_BUILD_TYPE=Release
cmake --build build/native-release --target air_llama --parallel 2
AIR_NATIVE_DIR="$PWD/build/native-release" cargo test --locked -p runtime-types -p llama-adapter
AIR_NATIVE_DIR="$PWD/build/native-release" cargo build --locked -p llama-adapter --example native-smoke
```

当前构建入口支持 Linux 开发验证和 Windows MSVC（目标验收以实际 CI / 设备记录为准）。Android 原生工具链集成属于 T07，本入口会明确拒绝，不能把 Linux 通过称为 Android 可用。

## 真实模型验证

```sh
cargo run --locked -p llama-adapter --example native-smoke -- \
  --model /path/to/locked.gguf --prompt-file tests/fixtures/chinese.txt \
  --mode generate --context-size 2048 --max-tokens 64 --repeat 2
NEXA_TEST_MODEL=/path/to/locked.gguf cargo test --locked -p llama-adapter \
  --test real_model -- --ignored --test-threads=1
```

`native-smoke --help` 列出取消、背压和预算模式；`xtask native-smoke` 驱动多场景并校验锁定元数据。默认单测不加载模型，显式忽略的真实模型测试覆盖 stop 命中、上下文超限、丢弃 prepared、取消/消费者停止/panic 后同一模型恢复及重复加载释放。仅测试通过不代表推理质量或目标设备性能验收。
