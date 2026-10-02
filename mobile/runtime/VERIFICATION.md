# T07-B1 Rust adapter 验证（2026-10-02 UTC）

状态：本目录实现和Linux研究/Android完整链接通过；生产模型准入、Android设备运行、Executor/store/UI不在本片完成范围。

## 固定身份

- Linux本轮实际编译器为Debian GCC14.2.0；另允许Ubuntu GCC13.3.0 CI构建profile，该profile尚待CI真实native/Rust门禁，不以允许列表充当验证证据
- Rust/Cargo 1.98.1；独立workspace只有mnn-adapter，metadata实际确认workspace_root为mobile/runtime
- MNN `d407447ed56c4121a11ccbd266dc184ca1ead0c2`
- patch set `43cc33146e2036ff452bd02d5ec352bb099d143ed4a4cdeb6ff55335987f9ce0`
- policy `ea06621b78e67e58f97f98951b26db0a8a893ded4112da3e5a762b98566fa328`
- frozen ABI1 header `40d1a79df99f5200d92e689baf791d5105d2da71d521fcd885a2807f0b916b8e`
- Linux shim archive `347849aa93bc5c222dbc7ed27e09775fc5e001e89bde11a867645c12655a49cc`；MNN archive `24d727baca3d63dc6e70589eef09910ec93142a2a0146ef0db0e19520831b1d6`
- Android shim archive `dbf3bdbd3e272735ba084a5543efd0be8329763b8a458b6ea1a4b6e74e49f5e0`；MNN archive `56b93cceed95a8e6cfdfb34814d779589accfbb09e6fa01f7caef7063b7a638b`
- 完整native manifest指纹：Linux `53eab05ec35465082f784af6e305635376e9e12f8284cc4d208d1f5f40448450`；Android `be2c62705f861ef6115d8a035fd1482662ccda0d11e8fbd642b4ca5b81676a72`。Linux实际build_identity输出与manifest SHA256一致；原始JSON包含全部archive/header/compiler/target字段，格式变化也保守改变指纹
- 候选五文件逐个size/SHA256复核通过；研究锁规范JSON摘要 `8d43d63204f753fd4a0b6b9089209f5641e206d5314c02a54186f17e556c1720`，不冒充未来生产包身份

## 实际运行及结果

以下命令均exit 0（负例runner内部要求被测cargo非零）：

1. `cargo metadata --manifest-path mobile/runtime/Cargo.toml --locked --offline --no-deps --format-version 1`：独立成员/根确认；根Cargo.toml/Cargo.lock `git diff --exit-code`无变化，两个lock所有共享依赖版本相同
2. `cargo test --manifest-path mobile/runtime/Cargo.toml --locked --offline`：6单测+4 compile-fail文档测试通过；真实测试默认显式ignored，未当作真实运行证据
3. `python3 mobile/runtime/scripts/test_real_model.py --model-dir VERIFIED_CANDIDATE --config CONTROLLED_RUNTIME`：真实测试显式运行通过，包含load取消/回调panic、callback嵌套load Busy、seed0 A→B→A、中文system/多轮、正常stop与取消/失败区分、精确模板预算/超1、源字符串已销毁后生成、template/tokenize/prefill/decode取消与每次恢复、基于phase的跨线程取消、text/progress panic失败及恢复、max1、close幂等/关闭后拒绝
4. Linux和Android分别 `cargo clippy ... --all-targets -- -D warnings`：通过；fmt --check通过
5. Linux `test_artifact_gate.py`：8项拒绝通过（ABI/target/patch/header/silent/compiler/污染archive/缺显式输入）。Android另加NDK版本/API错配，共10项拒绝通过
6. `tests/abi_layout.c`：Linux C11 `-Wall -Wextra -Werror`编译并运行；Android NDK clang API28编译对象通过，与Rust结构size/offset断言一致
7. `cargo test --no-run --target aarch64-linux-android --manifest-path mobile/runtime/Cargo.toml --locked --offline`：最终两个测试ELF与build_identity示例ELF完成Rust→shim→MNN→静态libc++/abi/unwind/builtins链接，非仅cargo check。NDK r30/30.0.16248370、Clang21、API28
8. `readelf -lW/-dW`：三个ELF均ARM aarch64、解释器/system/bin/linker64，全部LOAD对齐0x4000且GNU_RELRO末端16KiB对齐，DT_NEEDED仅libm.so/libdl.so/libc.so

最终Rust链接显式设置max-page-size=16384与common-page-size=16384，纠正早期仅检查LOAD对齐而GNU_RELRO仍4KiB的问题；不把早期LOAD结果算作完整16KiB门禁。

初次Android门禁正确拒绝NDK的libatomic.a纯注释兼容占位；native导出改为真实libclang_rt.builtins-aarch64-android.a（manifest名clang_rt_builtins）后重试成功，没有放宽非ELF archive校验。

## 开发环境原始证据

本轮未把模型或构建大文件纳入仓库。原始日志位于开发机 `/workspace/shared/`：

- `nexa-mnn-rust-test.log`、`nexa-mnn-rust-real-test.log`
- `nexa-mnn-rust-build-negative.log`、`nexa-mnn-rust-android-build-negative.log`
- `nexa-mnn-rust-android-link.log`、`nexa-mnn-rust-android-elf.log`
- `nexa-mnn-rust-metadata.json`、`nexa-mnn-rust-build-identity.log`

## 限制

- 没有Android真机执行，不授予生产validated、取消时延目标、Android日志/内存稳定性或APK完成结论
- 直接config入口仅可信研究组合；真实hash runner不提供生产资产闭包/TOCTOU/私有目录租约机制，后续store承担
- callback panic通过catch_unwind转明确失败；宿主panic hook仍会执行，panic=abort不能恢复。无全局hook替换
- callback内Drop另一个模型/Prepared按不可重入门禁拒绝并泄漏，须保留句柄至callback返回后close；这是保守失败边界，不是无泄漏承诺
- 未stage、commit、push；native目录由独立owner维护，Rust本片没有修改它们或根Cargo文件

## T07-B2 实现与验证（2026-10-02 UTC，独立于上述 B1 历史记录）

范围：新增 `mnn-model-store`、`mnn-executor`，修改独立 mobile workspace/lock/README；未改 adapter、native、Windows DTO、根 Cargo 或 shared core/types。实现真实私有 copy → snapshot → core → owner → MNN 链，不是 mock 生成。生产 Android 准入仍为空，合法候选被生产 resolver 明确拒绝。

### 固定身份与研究准入

- 五文件锁直接编译自 `scripts/android_mnn/candidate-model.json`，没有复制第二份锁；完整模型包 identity SHA256 为 `1ec59d439451738b4992f2ea5b06438788d752da81d55f11e1e8866d03fa7a57`
- Linux 实际 native artifact.json SHA256 `53eab05ec35465082f784af6e305635376e9e12f8284cc4d208d1f5f40448450`，compiler `c++ (Debian 14.2.0-19) 14.2.0`；其余 engine/patch/policy/header 同 B1
- 另由主代理独立审核 [c651 B1 CI 的 Linux 阶段证据](https://github.com/Naza3/Nexa/actions/runs/36966789118)，准许 test-only Ubuntu 记录：compiler `g++-13 (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0`，artifact SHA256 `88c287f25394d6b4565d108592947c1973e7739187ff93e22a45391941450adc`。该作业 Linux native/real/adapter 阶段通过，整个作业随后因 Android exporter 的 libunwind 路径失败；绝不表述为整 job 或 Android 通过。B2 新 Ubuntu CI 尚待执行，重建不匹配该指纹必须失败，不能动态接受当前 hash
- `ResearchCpuEvidence`、pressure/progress 注入只在 executor `#[cfg(test)]` 单元测试目标；比较完整 BuildIdentity 六字段与固定模型 digest。生产没有 feature/env admission 入口；`nm -C` 生产 rlib 未发现 ResearchCpuEvidence、PressureSink、ProgressHook 或真实测试符号
- 首片 context≤2048、threads 1..2、chunk 1..128，由组合显式传参；本轮实际线程 2、chunk 32，等预算测试另用固定 prompt 的精确小 context。不沿用 core 默认 batch 512，也不静默夹限

### 实际命令与结果

环境先 `source /workspace/shared/nexa-tools/env.sh`，Linux 使用独立 `/workspace/shared/nexa-mnn-b2-rust-target`；`NEXA_MNN_TEST_MODEL` 指向只读原始五文件目录，`NEXA_MNN_ARTIFACT_DIR` 指向上述锁定 Linux artifact。所有写模型/篡改均发生在新建临时 0700 store，不修改源候选。

1. `cargo test --manifest-path mobile/runtime/Cargo.toml --locked --offline --workspace`：exit 0；adapter 6、executor 2、store 16 普通单测通过，adapter 4 compile-fail doc tests 通过；5 项真实测试默认 ignored（B1 1 + B2 4），不把 ignored 记成真实通过
2. `cargo clippy --manifest-path mobile/runtime/Cargo.toml --locked --offline --all-targets -- -D warnings`：exit 0；定向 fmt 通过
3. `cargo test --manifest-path mobile/runtime/Cargo.toml --locked --offline -p mnn-model-store -p mnn-executor -- --ignored --test-threads=1`：exit 0；executor 三项真实门禁 195.00 秒，store 一项真实门禁 38.60 秒；新增 Drop 断言的同名精确单项随后复验 exit 0、114.97 秒，见 final 日志
4. 独立 metadata 与 lock 比较：mobile workspace_root 正确，根成员无 MNN；所有共同 package/version 精确匹配根 lock，未升级既有 B1 依赖。根 Cargo.toml/Cargo.lock/runtime-core/runtime-types diff 为空
5. Android arm64 `cargo test --no-run --target aarch64-linux-android -p mnn-model-store -p mnn-executor` 已完成两新增 lib-test ELF 的真实全链接；目标目录 `/workspace/shared/nexa-mnn-b2-android-target`，NDK r30/API28 与 B1 artifact。最终源码全 workspace 复链接 exit 0，实际恰好 5 个 ELF（原 B1 三个 + 新 B2 两个）；所有 LOAD 对齐/offset-vaddr 同余及 GNU_RELRO 尾端均通过 16KiB 检查，见 android-final/ELF 报告。Android artifact SHA256 `be2c62705f861ef6115d8a035fd1482662ccda0d11e8fbd642b4ca5b81676a72`；不执行为设备测试

新增直接依赖精确版本：serde 1.0.229、serde_json 1.0.151、sha2 0.10.9、fs2 0.4.3、tempfile 3.27.0、libc 0.2.189；root path runtime-types/runtime-core 0.1.0。全部与根 lock 已有版本一致；完整共享版本表在 isolation.json。

### 断言覆盖与限定

- strict duplicate key（含嵌套）、未知身份/执行字段、尾随 JSON、超大 JSON、固定 path/role/size/hash/template/source/policy 变更，即使重算自证 digest 仍拒绝
- 私有目录与跨实例独占锁、硬链接/符号链接/祖先链接、额外 context 文件/子目录/设备、缺文件、源 hash/长度、分块取消；空间失败是显式 ENOSPC 注入，不声称真实填满磁盘。沙箱不允许创建测试 Unix socket，最终测试用已有设备 `/dev/null` 检查普通文件门禁，未声称 socket/FIFO 的实际构造覆盖
- 原始五文件真实 copy/hash、copy 中断后 staging 清理、重开、snapshot/load lease 阻止删除、manifest 路径/不同 generation/额外文件/私有 copy config/symlink/hardlink 篡改拒绝，独立重建配置的 CPU/高精度/线程/nonthinking 字段断言
- 发布 rename 成功后父 fsync 的 EIO/ENOSPC，必须登记后 poison、禁止重复发布/新 snapshot；rename 失败保留可重试状态。删除先 tombstone；删除 rename、fsync、部分删除和清理后 fsync 故障都可安全重开。未知或 symlink tombstone 不被删除。该层用只含固定目录/manifest 形状的微型事务 fixture，不把它们作为可推理资产
- 真实 core load、英文/system/中英多轮、同 prompt 精确 context 等式与超 1 拒绝、用户 stop、decode 后取消/准确 usage/再次生成、断流恢复、卸载与 shutdown
- 背压为明确合成压力：真实 MNN 首个 text callback 内直接填满 core 的 256KiB 原账本，保留 4KiB 分片；未加第二文本队列。默认 10 秒无消费时限、已阻塞 callback 的取消唤醒（测试界限 <3 秒）、断流、真实无压力恢复及 shutdown 单终态断言通过；不宣称自然输出了 256KiB
- fatal callback-panic 在真实 native progress 回调中注入，adapter 捕获并返回故障，owner 确认释放后单发 Faulted；core 拒绝继续生成，显式 reload 恢复。不是任意 kernel crash/abort 的隔离证据
- load deadline 用测试屏障保持真实 native Load checkpoint，超过 30 秒 core deadline 时仍不提前回复清理完成；释放屏障后安全返回 LoadTimeout。此前测试设 10 秒，但 debug 大权重重 hash 与其他测试竞争使取消先发生在 hash 阶段、90 秒等待屏障失败，已保留为测试时序修正，不修改产品 timeout 或伪造 native 停止
- 专用 Drop 竞态复验：真实模型 loaded 时 close 拒绝；生成停在真实 Prefill checkpoint 时 close 仍拒绝，Drop <1 秒返回且 generation 仍被 lease 固定；释放 checkpoint 并 join 测试持有的 owner handle 后才允许删除。这个 join 属于测试观察，不在产品 Drop 内
- 初版等预算测试 prompt 不足 core 最小 32-token context，正常触发 InvalidArgument；已换较长固定合成 prompt 后精确预算通过，不放宽 core 参数范围

### 未覆盖与失败语义

没有 Android 设备、App 生命周期、GPU/NPU、长期内存/耗时分位数、真实磁盘掉电/全盘耗尽、kernel abort 恢复证据。私有 copy/lease/重 hash 保障外部来源和正常 App 并发，不抵御同 UID 恶意进程。

正常成功/取消/可恢复失败的 usage 来自 adapter 已知计数。现有 `ExecutorEvent::Faulted` 不带 usage；不可恢复错误的最终 completion 计数无法交给共享 core，终态中的默认 0 不代表精确计数。按已审定边界仅发一次 Faulted，禁止先 GenerationFailed 再 Faulted，未改 Windows DTO。cleanup 无法确认时永久不可用并保守 pin lease，绝不假 Unloaded；Drop 不强杀线程/异线程释放 native。宿主 panic hook/abort 限制沿用 B1。

本地原始证据目录：`/workspace/shared/nexa-mnn-b2-verification/`，含 unit.log、unit-final.log、clippy.log、real.log、fault-timeout-drop-final.log、mobile/root-metadata.json、isolation.json、production-build.log、production-symbols.txt、后续 android-final/ELF 报告。未 stage/commit/push，由主代理整合。本文 B1 历史段中的“只有 adapter/Executor 未完成”等仅描述 B1 当时范围，以本节为 B2 增量事实。


## 同次研究凭据改造（2026-10-02，恢复后本地完整链通过）

原有历史结果属于对应已冻结源码。凭据实现新增executor cfg(test)私有消费者及dev-only serde/sha2/libc，生产没有receipt入口；同批次另包含root审查并实现的cleanup-unconfirmed永久pin修复，不能泛称整个批次无生产变化。

本次重新验证：34项Rust单元、4项compile-fail、fmt/clippy通过；包括7项receipt格式/文件/身份/命名空间测试、缺坏凭据不静态回退的子进程测试及2项生产cleanup回归。helper为85 pass/1 actionlint缺工具skip。独立只读审查无P0/P1阻塞，并单独重跑receipt Python 11/11。

独立新work实际通过12阶段，包含七个真实B1前置→原子只读receipt→四项显式B2，以及Android native/Rust/五ELF门禁。`stage_reports`真实复验得到`evidence_verified=true`、missing/invalid为空，七份proof及receipt上传原始字节SHA完全一致。该上下文为`local-verification/source_clean=false`，故clean CI总门禁按预期非零且`all_required_steps_succeeded=false`；不是GitHub成功。

Linux完整manifest为`b8b4d8efb06388f49c6457d177997f2bf630c5dceeb9ec190d3cc245a313372d`，Android为`6ff7a9625cd8bf1135e3f82fa36095da4f4f27ebc0ac808c474020e62e1cb750`；receipt为`eff52f1647843347110100c704812b900e39406ce2d22c26b2b7da3f262a6e6b`。执行时源码范围快照`8f618bd1c220095e62182a3ee29f1303fb0eeb8f470fc3f914e630b679049c6a`，之后仅补写本记录。本地证据`/workspace/shared/nexa-receipt-complete/complete-verification.json`及`verified-upload/`。

本地复用Linux Makefiles/Android Ninja缓存但重跑全部build/audit/hash和真实门禁；NDK ZIP哈希引用本环境恢复时的校验，本轮重新验properties/clang/五模型文件。容器ptrace使LSan不可用，本地只禁leak扫描，ASan/UBSan仍实际运行。GitHub路径保持全新Ninja构建和ZIP实际下载/hash。生产rlib实际无receipt/test工厂符号或相关环境变量字符串，normal直接依赖不变、registry版本与checksum未升级。

凭据不证明产品支持或防伪认证。最终仍需独立核验精确新GitHub commit/run/outcome/产物；Android仅完成链接/静态审计，未执行设备测试。完整边界见[CI记录](../../scripts/android_mnn/B1_CI.md)。
