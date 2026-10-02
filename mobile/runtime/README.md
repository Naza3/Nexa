# MNN 移动 Rust workspace（T07-B1/B2）

独立 `[workspace]` / Cargo.lock / Rust 1.98.1，包含 `mnn-adapter`、`mnn-model-store`、`mnn-executor`。
根 Windows workspace、Cargo.lock、公共 DTO 和 llama 依赖图不变。本目录不包含 Flutter/App；Android 生产设备准入仍为空。

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


## T07-B2：受控 store 与共享 core

`mnn-model-store` 只接收根 `scripts/android_mnn/candidate-model.json` 锁定的 Qwen3-0.6B 五文件，不是通用 MNN importer。公开预转换来源、固定 revision、五文件 size/hash、角色、模板、CPU/nonthinking 策略进入规范化 identity；`display_name`、安装路径、缓存和准入不进入模型摘要。exporter、original revision、导出参数明确为 unknown/null，不阻止此固定输入的运行研究，也不声称能重现转换。

- `MnnModelStore::open` 要求已存在的绝对、无 symlink 祖先、0700 App 私有目录；不要传用户共享下载目录。Android App 如取得别名路径，应先从可信 App context 获取规范化私有路径
- `import_candidate(source, cancel)` 显式读取五文件，拒绝额外文件、目录、设备、symlink/hardlink、路径或身份变更；64KiB 分块 copy/hash，每块检查取消。源目录永不原地修改、零复制或链接复用
- 私有 staging 的文件与目录 fsync 后原子 rename 为不可变 generation。rename 后先登记、再 fsync 父目录；后者失败会 poison 当前 store，所有新 snapshot/导入/删除均拒绝，必须释放旧句柄后重开核验，不能误称未发布而重试
- `snapshot()?` 是有限元数据查询，固定 generation 路径；snapshot 或 load lease 存活时不得删除该 generation。独占文件锁跨 store 实例/进程保持到最后一个 generation 引用释放
- 删除先原子 rename 为专用 tombstone、移除登记、fsync，再清理。部分删除或 fsync 失败 poison；重开只恢复精确受控命名、0700、非 symlink 的 staging/work/tombstone。未知目录 fail closed，不猜测删除
- load owner 重新核对 manifest、完整文件表/hash、重复 JSON key、固定 config/metadata 白名单、模板 hash 和 checked embedding spans。独立私有 work 目录从白名单元数据重建运行配置，绝不直接信任外部 runtime config。包目录只含五文件与 manifest，无 context/辅助图闭包旁路

`MnnExecutor::composition(store.snapshot()?)` 一次返回绑定同一 Arc 注册表的 resolver/executor。`ResolvedModel.path` 是实际 `manifest.json` 路径，禁止 JSON/URL 编码；load 必须匹配工厂持有的 id、generation 路径与 context。完整哈希在 owner 线程，resolve 不 hash 大权重。移动调用方必须显式使用合法设置，例如：

```rust
let options = runtime_types::LoadOptions {
    context_size: 2048,
    threads: 2,
    batch_size: 32,
};
let config = runtime_types::RuntimeConfig {
    load_options: options,
    ..runtime_types::RuntimeConfig::android()
};
let (resolver, executor) = mnn_executor::MnnExecutor::composition(store.snapshot()?)?;
let runtime = runtime_core::Runtime::spawn(config, resolver, executor)?;
```

这段是生产组合方式，**目前生产 resolver 必须拒绝加载**：固定包 hash 正确仍无 Android 设备受信证据。`resolve_candidate` 永远返回 `validated=false`。仅 executor 的 `#[cfg(test)]` 单元测试工厂持 `ResearchCpuEvidence`，核对受审 Linux 完整 BuildIdentity（包括 artifact.json 字节摘要）、固定模型/模板/策略后，才可在该测试域接 core；无生产 feature、环境开关或持久 manifest validated 字段。直接 adapter/Executor 是低层可信研究边界，不能当作产品准入证明。

### 生命周期、故障与现有 DTO 限制

owner 专用线程持 Model/Prepared/lease；容量 1 mailbox 加单活跃操作原子门禁，不另建调度器或文本队列。取消句柄同时设置独立 native flag 与 hash 取消标志，不排在生成后面。真实 text callback 直接使用 `ExecutionEvents::text_delta`，共享 core 的 256KiB 账本、4KiB UTF-8 分片及默认 10 秒无消费进展时限。

正常成功、取消、可恢复参数/预算或显式 callback 失败传准确已知 usage。不可恢复 native/protocol/callback-panic 错误在确认 owner 清理后只发送一次 `Faulted`，需显式 reload；不能先发送 `GenerationFailed` 再发送 `Faulted`。现有 core `Faulted` 事件没有 usage 字段，因此这条故障路径只保留已知 Prepared prompt 计数，最终 completion 计数无法传入共享终态；其默认零值**不是精确失败用量证明**。本片不为此修改 Windows 共用 DTO。

清理不确认时发送 `CleanupUnconfirmed`、永久拒绝新操作并保守固定 lease/句柄；不虚报 Unloaded。close 仅在空闲且已卸载后关 mailbox/join；Drop 只取消并关发送端，不 join 卡住的 kernel，也不在调用线程销毁 native。owner 返回后自行清理，仍不可抢占 kernel。测试 hook/压力注入均不进入生产符号。Rust panic hook 仍可能运行，继承 B1 限制；不抑制宿主日志。

### B2 显式验证

纯 store 测试不需要 native artifact：

```sh
cargo test --manifest-path mobile/runtime/Cargo.toml --locked --offline -p mnn-model-store
```

实际 copy → store → executor → core 使用固定真实五文件源目录，只在新临时私有 store 工作：

```sh
export NEXA_MNN_TEST_MODEL=/absolute/fixed-five-file-candidate
export NEXA_MNN_ARTIFACT_DIR=/absolute/audited-native-artifact
export CARGO_TARGET_DIR=/absolute/mobile-rust-linux-target
cargo test --manifest-path mobile/runtime/Cargo.toml --locked --offline \
  -p mnn-model-store -p mnn-executor -- --ignored --test-threads=1
```

四项默认 ignored 门禁及精确名称在两个 crate 的源码中；没显式运行不算真实验证。本地无凭据时只接受测试源维护的明确受审 Debian 完整 native fingerprint。CI 改为消费同次七个真实 B1 前置生成的45分钟内只读研究凭据；缺失/损坏不得静态回退，不接受单独 build_info 自授。凭据与环境变量只编译于 executor 的私有 cfg(test) 模块，生产 resolver 无入口；详见 [CI门禁](../../scripts/android_mnn/B1_CI.md)和 [ADR0012](../../docs/decisions/0012-ci-research-evidence-receipts.md)。CPU/多轮/等预算与超 1/stop/取消恢复/断流/故障 reload/慢消费/close/Drop 是不同断言。背压测试在**真实 native 首个 text callback**内注入明确合成数据占满 core 原账本，不声称自然生成了 256KiB；默认 10 秒真实等待与取消唤醒分报。load-timeout 用真实 native checkpoint 的测试屏障证实 deadline 不提前确认清理；并非测得 30 秒 native 内核耗时。

Android 新增两个 lib-test ELF 需完整交叉链接和 16KiB LOAD/RELRO 检查；交叉链接、Linux 实测、目标设备运行分开记录。独立依赖与符号检查要求 root workspace 无 MNN、所有共享依赖版本与根 lock 精确相同，生产 rlib 无 ResearchCpuEvidence 或测试压力入口。


同次研究凭据已在2026-10-02的本地完整12阶段中实际完成B1→receipt→四项B2，并重链/检查五个Android ELF；
最终proof/receipt上传原字节摘要复现。该证据明确为local dirty工作区，clean CI总门禁按预期拒绝，
不能当作新提交GitHub成功或Android运行。独立只读审查及receipt 11项Python测试通过；
范围与本地限制见[验证记录](VERIFICATION.md)和[CI记录](../../scripts/android_mnn/B1_CI.md)。
