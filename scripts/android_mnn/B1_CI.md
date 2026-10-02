# T07-B1/B2 可复现 CI

任务状态：保留全部B1门禁；本轮B2扩展已实现、待完整GitHub CI验证。`.github/workflows/android-mnn-native.yml`
使用独立 Ubuntu 24.04 / GCC 13.3.0 候选配置；允许该配置不等于已通过 CI。
固定 CMake 4.4.3、Ninja 1.13.2、Rust 1.98.1 及 Android Rust target。
显式 `inputs` 阶段复用 T07-A 的 size/SHA 锁定 NDK r30 与五文件模型下载，
随后获取独立 mobile Cargo.lock 的依赖。无新增 SDK 安装或协议接受；已有 NDK
许可审查是前提。此后 Cargo 命令均为 locked/offline。

作业上限 45 分钟，编译最多 2 jobs。每条命令另设超时；超时终止整个子进程组，回收最多等待5秒。清理未确认时标记
`cleanup_unconfirmed` 并阻止后续阶段，不宣称已结束。私有命令日志使用16MiB软阈值，
每50ms轮询且进程快速退出后再次检查；这不是硬字节上限。确认写入停止后截断超限部分，
任何一次读取最多256KiB。
以下阶段及完整证据均为强制门禁：

1. 实际工具/编译器版本、clean commit/tree 身份、helper 测试及固定notice库存hash验证；
   notice仅证明源码库存完整，不代表最终APK/.so闭包或许可UI完成
2. 显式固定输入与锁定 Cargo 依赖获取
3. pristine 上游到独立私有 patched 副本，精确 postimage 复核
4. 独立未补丁 T07-A Linux 探针与 CTest，并复核原始源码仍 clean
5. Linux C ABI CTest（ABI/sampler、stream、logging macro）、实际编译/头依赖
   日志审计、真实 archive 导出、仅 stream 的 ASan/UBSan
6. 原生真实请求、全部数字取消检查点、与 pristine 探针精确对照、8 项负例
   privacy canary 及成功中英/多轮/取消/callback/恢复 canary
7. Rust fmt、clippy、unit、compile-fail、artifact 负例、固定真实模型测试，
   实际 C 头布局编译并运行，运行 build_identity 示例逐字段对照实际 manifest；
   显式运行B2 store/executor四项ignored真实测试，并核对实际执行结果
8. Android native 交叉构建、日志审计、真实 shim/MNN 与固定
   libc++/c++abi/unwind/Clang builtins 静态闭包导出
9. Android clippy、Rust 最终测试 ELF 完整链接、artifact 负例、C 头布局交叉编译
10. Cargo 实际报告的五个固定最终ELF（adapter/模型store/executor三个lib-test、real_model及build_identity示例）：AArch64 PIE/linker64、依赖仅 libc/libdl/libm、
    全部 LOAD≥16KiB 且地址/偏移同余、无 WX 段/可执行栈、非空 GNU_RELRO
    结束地址按 16KiB 对齐

Rust Android 链接同时设置 max-page-size/common-page-size=16384。
仅 LOAD 对齐不足以证明 RELRO 边界。使用 Cargo JSON 的 compiler-artifact
executable 事件定位五个固定最终ELF（adapter/模型store/executor三个lib-test、real_model及build_identity示例），不猜测 hash 文件名、不以 cargo check 替代链接。
不执行 Android 测试，不声称设备运行。

## B2 显式真实门禁及研究准入边界

新增workspace成员为`mnn-model-store`、`mnn-executor`。默认`cargo test`仍执行所有
非ignored单元测试，但不能把默认跳过的真实测试当作通过。本CI先分别用
`cargo test -p PACKAGE --lib -- --ignored --list`精确核对以下清单，再逐项调用：

```sh
cargo test --manifest-path mobile/runtime/Cargo.toml --locked --offline \
  -p PACKAGE --lib EXACT_NAME -- --ignored --exact --test-threads=1
```

- `mnn-model-store`：`negative_tests::real_store_lease_reopen_tamper_and_interruption`
- `mnn-executor`：`tests::real_store_core_lifecycle`
- `mnn-executor`：`tests::real_owner_backpressure_cancel_disconnect_shutdown`
- `mnn-executor`：`tests::real_owner_fault_reload_load_timeout_and_idle`

每项必须实际出现准确test名称和恰好`1 passed / 0 failed / 0 ignored`；零测试、
遗漏、改名、新增未审ignored测试或重复输出都失败。模型输入只设置为显式setup已
锁定的五文件候选目录，所有篡改场景仅操作private copy。矩阵前后均重新hash原候选。
受控panic仍由测试捕获，命令原始输出只留private日志；上传固定case标签和通过状态。

研究准入仍只存在于executor的`cfg(test)`代码：由审查后的精确B1 Linux原生产物
manifest指纹、编译器/target及固定候选身份决定。CI不通过环境变量动态授予研究或
生产准入，不填猜测指纹。Ubuntu13.3配置须先有完整通过的独立B1 Linux native/adapter证据并经主代理审查；
实际manifest发生改变时失败并重新审查，不自动把当前产物加入允许列表。
B2报告始终明确`research_only=true`和`production_admitted=false`。

Android Cargo事件对五个目标逐个验证package ID/版本、源码入口、kind、crate_types、
profile.test和可执行文件路径属于该Android target输出目录；缺少、重复、替换、
额外目标或宿主ELF路径均拒绝。每个已链接ELF都执行原有16KiB/RELRO/依赖门禁，
不执行Android测试。共享`runtime-core`/`runtime-types`及根Cargo输入现也触发此
Android workflow，避免B2路径依赖变更绕过验证；Windows workflow未作修改。

## 证据及失败保留

`b1_ci.py` 将命令输出捕获到已忽略的 `build/t07b1` 私有日志。
运行 config、合成 prompt/回答/token vector、原始对照报告也仅保留在那里。
native export 生成实际本地 artifact manifest/archive，交给 Rust 严格消费。

上传仅包含重新构造、严格 schema 校验的 JSON：固定检查名、布尔值、计数、数字
采样延迟、源码/patch/policy/header/model/NDK/archive/编译器身份摘要及已验证
ELF 属性。完整实际编译器字符串保留在本地 artifact manifest；上传编译器 SHA256
与固定工具配置。另保留实际 artifact.json 完整字节 SHA256，绑定完整构建闭包。
编译/链接失败额外保留最多16行、合计4096字节的错误诊断，去除绝对源码/工作区/home路径；
仅编译命令可生成此字段，不接受真实模型、panic或聊天输出。其余不上传原始日志、正文、模型、SDK、静态库或可执行文件；原生二进制分发仍等待
license/notice 闭包完成。

staging 在失败后仍运行，保留command index/category/timeout/timed_out/log_limit/cleanup状态、退出码与固定失败标签；
对于已知privacy case、upstream比较字段、candidate/source/export错误仅记录固定标识，删除未经验证的部分嵌套
报告，记录 missing/invalid 报告。只有所有必需步骤/报告成功、源码 clean 且身份
不变时才允许总体成功。完整性门禁失败后仍上传可用安全证据。外部取消可能直接终止
runner，不能保证这种情况下仍有机会完成清理/上传。取消延迟仅为 Linux
checkpoint 到安全返回的样本，不是任意时刻或 Android 真机的取消上限。

## 触发隔离

Windows `paths-ignore` 仅排除明确孤立路径：
`native/mnn-{probe,shim,patches}/**`、`scripts/android_mnn/**`、`mobile/**`、
两个 Android workflow、`docs/**` 及明确列出的根文档。
GitHub 仅当**全部**改动匹配排除列表时跳过；混合 Android/docs 与 Windows 代码
的提交仍运行 Windows。`packaging/**`、`crates/**`、`native/llama-shim/**`、
根 Cargo/CMake 输入、Windows workflow 以及未知新路径均仍触发。
此前B1提交的 legacy UTF-8 修正涉及 `native/llama-shim/**`，因此该次B1提交仍运行Windows。本轮B2未修改Windows workflow。
没有修改 Windows permissions、timeout、job 或验证门禁。

## 实际本地验证

2026-10-02 执行以下命令，均 exit 0：

```sh
python -m unittest discover -s scripts/android_mnn -p 'test_*.py'
actionlint .github/workflows/android-mnn-native.yml .github/workflows/native-windows.yml
```

Python 共 71 项（1 项已有环境条件 skip）；包括缺失/失败/非法报告、嵌套原文泄露、
重复 outcomes、dirty source、遗漏取消阶段、非有限时间、错误 ELF 架构/依赖、
不足 LOAD/RELRO 对齐及可执行栈，另覆盖快速日志超限、读取上限、超时回收、kill/退出竞态、
无法确认回收后的阻断，以及编译错误脱敏/真实模型输出禁止进入诊断。另断言 Windows 精确排除列表、全部已知 Windows
输入与未知路径仍触发。现有真实 native 日志的 10 个取消样本也经新 parser 复核。
本轮未重新编译native或执行B2真实模型/Android构建；CI辅助测试之外，使用现有NDK与实际Android build重新执行export。
notice verifier实际通过12个component、27份hash库存文件。下一步由主代理审查后运行完整工作流。

## NDK r30静态运行库路径修正与实际验证

c651的CI失败定位为`export_archive_missing`：native compile/audit已成功，但原helper
错误地从sysroot读取libunwind。现通过固定NDK的`clang++ --print-resource-dir`获取
resource目录，并要求其精确为同一toolchain下的`lib/clang/21`：

- libc++/c++abi：`sysroot/usr/lib/aarch64-linux-android/`
- unwind：`lib/clang/21/lib/linux/aarch64/libunwind.a`
- builtins：`lib/clang/21/lib/linux/libclang_rt.builtins-aarch64-android.a`

不使用glob，不接受其他Clang版本、musl/host替代路径或symlink重定向。导出前逐个
读取全部archive member的固定长度ar/ELF头，验证ELF64 little-endian及目标架构；
不将archive整包读入内存。缺库报告新增`missing_archive`字段，仅允许六个固定逻辑
库名之一，不能带绝对路径。已有命令诊断/日志隐私限制保持不变。

2026-10-02实际调用修正后helper的Android export，重新执行source身份核验、
编译闭包审计及六库导出，exit 0。对象数为shim=1、MNN=444、libc++=54、c++abi=18、
unwind=9、builtins=287；实际manifest SHA保持
`be2c62705f861ef6115d8a035fd1482662ccda0d11e8fbd642b4ca5b81676a72`。
此摘要仅记录本地实际产物，不硬编码为CI应得摘要。未重新编译、未声称Android设备运行。
缺库、错误架构、错误resource版本、musl/symlink错路径和缺库名安全输出均有回归测试。
