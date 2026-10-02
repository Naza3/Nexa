# MNN 移动 Rust workspace（T07-B1）

独立 `[workspace]` / Cargo.lock / Rust 1.98.1；当前只有 `mnn-adapter`。
根 Windows workspace、Cargo.lock、公共 DTO 和 llama 依赖图不变。没有 Executor、store、Flutter 或 APK。

## 构建与真实验证

`mnn-adapter` 只消费本地已构建的真实 shim，绝不下载或自动生成假 backend。
先按 `../../native/mnn-shim/README.md` 构建并导出 artifact。

```sh
export NEXA_MNN_ARTIFACT_DIR=/absolute/native/artifact
export CARGO_TARGET_DIR=/absolute/mobile-rust-linux-target
cargo test --manifest-path mobile/runtime/Cargo.toml --locked --offline
cargo clippy --manifest-path mobile/runtime/Cargo.toml --locked --offline --all-targets -- -D warnings
python3 mobile/runtime/scripts/test_artifact_gate.py
cargo run --quiet --manifest-path mobile/runtime/Cargo.toml --locked --offline --example build_identity
python3 mobile/runtime/scripts/test_real_model.py \
  --model-dir /absolute/verified-candidate \
  --config /absolute/controlled-runtime.json
```

最后一条会核对仓库候选五文件的长度/SHA256、运行 config 的 base_dir，然后执行显式 ignored 的真实模型集成测试；缺文件/配置直接失败。研究输入身份为候选锁完整 JSON 的 sorted-key/compact UTF-8 SHA256，**不是**未来生产 package manifest 身份/准入证据。测试只使用合成输入；不打印 prompt 或生成文本。测试 panic 故意触发，Rust 默认 panic hook 可能打印固定测试字面量。

build.rs 严格核对 artifact.json 的 ABI、目标、Release、silent_logs、Rust1.98.1、Linux Debian GCC14.2.0开发profile或Ubuntu GCC13.3.0 CI profile/Android NDKr30 Clang21 API28、repo精确commit/patch/policy及头文件SHA256、每个archive摘要/ELF64架构与受限系统库。静态库仅允许产物目录内的相对路径。Android要求显式包含已hash的 libc++_static；Linux开发只支持 x86_64-unknown-linux-gnu，Android只支持 aarch64-linux-android。Ubuntu GCC13.3.0仅列入允许构建profile，必须由对应CI执行真实native/Rust门禁后才能声称该profile通过；本地已验证的是Debian GCC14.2.0。完整实际编译器版本仍保存在artifact.json，native导出器与CMake真实compiler逐字比对。manifest与库的本地构建环境必须可信；摘要检查不是签名/供应链认证。build_info在实际加载前再次校对实际链接的shim身份。`BuildIdentity.artifact_manifest_sha256`是所有校验通过的原始artifact.json字节SHA256，绑定包括shim archive内容在内的全部字段；仅修改shim而不修改MNN patch时也必变。另公开target/compiler；示例输出固定key=value供CI保存。该指纹不是模型校验/生产准入，也不代替应用/Rust源码commit身份。

Android使用独立NDK r30/API28 clang linker、native artifact目录和Cargo target-dir；例如环境变量 `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` 指向 `aarch64-linux-android28-clang`，同时设置 `CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"`，然后运行 `cargo test --no-run --target aarch64-linux-android --manifest-path mobile/runtime/Cargo.toml --locked --offline`，使最终测试可执行文件完成真实链接。两项page-size必须同时指定并验证LOAD对齐与GNU_RELRO末端；本crate的build.rs也为其最终bin发出同样参数，下游应用不要假设rlib会传播link-arg。交叉构建不等于设备运行或生产准入。

## 安全边界

- Model / Prepared 都是 !Send / !Sync；Prepared独占借用Model；generate按值消耗Prepared，仅一次
- 所有模型操作受线程局部不可重入门禁保护，包括在另一模型callback里调用load/close。Cancellation独立Arc只包装原生atomic flag，唯一允许跨线程的控制对象；每次操作持clone，最后引用才destroy
- 请求字符串只在prepare调用内借用，native同步复制；progress和text userdata仅当前同步调用有效，没有保存Rust引用
- 所有callback catch_unwind；panic请求cancel并返回明确CallbackPanic，不冒充正常用户取消；不替换全局panic hook。catch_unwind无法阻止宿主panic hook日志，panic=abort无法恢复
- text须1..4096 bytes有效UTF-8；不lossy解码。不累计输出；调用者自行实现有界背压。回调必须及时返回，不能阻塞等待同一owner操作
- generate失败仍携带已知prompt/completion计数、resolved seed和终止原因。callback取消、正常stop、EOS、length和错误分开。固定seed=0有效；Fixed(u32::MAX)拒绝，Random显式映射该sentinel
- `close(&mut self)`报告错误并保留句柄以便owner稍后重试；Drop只能在owner安全析构，失败则泄漏而不强释放/错误线程销毁。callback内部Drop另一个Model/Prepared会因重入被拒绝并泄漏，应在callback返回后显式close
- 原生kernel不可抢占；cancel发出不等于已安全停止。直到同步调用返回才有清理完成依据

这是**可信研究组合入口**。LoadOptions的artifact_sha256只是身份传递，adapter不检查模型文件hash、config引用闭包、TOCTOU、私有存储租约或生产准入。调用者必须提供受控不可变配置/资产；实现生产store并通过设备证据前，不能把任意外部config或目录当成安全产品输入。

## 已覆盖测试

实际C头布局断言位于 `crates/mnn-adapter/tests/abi_layout.c`，分别用Linux cc和NDK API28 clang编译；Rust布局测试与之对应。

真实native build-info/ABI布局、参数有限数值与边界、Arc跨线程取消、回调UTF-8/块大小及panic、不可重入、4个compile-fail所有权例。
真实候选集成：seed0 A→B→A、中文system/多轮、load/template/tokenize/prefill/decode取消与恢复、基于phase的跨线程取消、callback正常拒绝/失败/panic、max1和显式close。具体运行证据由本轮verification记录汇总；这些测试不代替Android设备或完整生命周期/长期内存门禁。
