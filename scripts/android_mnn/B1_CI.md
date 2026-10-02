# T07-B1/B2 可复现 CI

任务状态：f4fa90 的完整 B1/B2 CI 已通过；本轮同次研究凭据已通过本地完整12阶段与独立只读审查，新提交的GitHub结果仍待验证。`.github/workflows/android-mnn-native.yml`
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
8. 校验七个前置的实际 outcomes、报告、当前源码/manifest/archive/模型身份，原子封存只读研究凭据和七份原始 proof
9. 在同一 context 下显式运行 B2 store/executor 四项 ignored 真实测试，并复验凭据与原报告
10. Android native 交叉构建、日志审计、真实 shim/MNN 与固定
   libc++/c++abi/unwind/Clang builtins 静态闭包导出
11. Android clippy、Rust 最终测试 ELF 完整链接、artifact 负例、C 头布局交叉编译
12. Cargo 实际报告的五个固定最终ELF（adapter/模型store/executor三个lib-test、real_model及build_identity示例）：AArch64 PIE/linker64、libc必需、libdl/libm可选且不得重复或出现其他依赖、
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

研究组合仅存在于 executor 的 `cfg(test)` 代码。`rust_linux` 先完成独立 B1 门禁，
`research_receipt` 必须检查 tools/inputs/patch/baseline/linux_native/native_real/rust_linux
七个真实成功结果，随后 `b2_linux` 才可消费短期凭据。禁止引用自身或 B2 报告形成循环证明。
CI 中缺失或损坏凭据直接失败，不回退静态白名单；本地无凭据时只保留明确受审的 Debian
完整身份记录。生产 resolver、模型 manifest 与公共 API 没有新增准入入口。

schema 2 阶段报告从生成时绑定 source commit/tree/clean、固定源码范围快照摘要、
run/attempt/job 与随机 context。凭据 `subject` 绑定完整 manifest、上游/patch/policy/header、
目标/精确 compiler/profile、候选锁文件字节摘要、store candidate identity 与 template 三种不同摘要。
七份原报告只读封存在固定文件集合，逐份完整字节 SHA256；凭据有效期最多45分钟。
消费者拒绝重复/未知字段、额外/遗漏 proof、错误时限、链接/越界、超限/可写/替换文件和混合 context。
每个 JSON 最多64KiB；Linux `renameat2(RENAME_NOREPLACE)` 保证新 bundle 不覆盖已有目录。
最终上传门禁再次比较 bundle、原报告、当前源码/原生产物以及 B2 凭据摘要。

这只是受审源码与可信 CI 执行链内防误混的测试编排，不是签名、供应链认证或抗恶意运行者机制。
最终研究结论仍需独立核验精确 GitHub commit/run/outcome/证据。`local-verification` 明确记录
工作区快照和 clean 状态，不能成为 GitHub/Ubuntu或产品准入证据。B2始终为
`research_only=true`、`production_admitted=false`、`android_run=false`。详见 [ADR0012](../../docs/decisions/0012-ci-research-evidence-receipts.md)。

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
ELF 属性。凭据仅允许精确审核的固定编译器字符串及其 SHA256；不包含主机路径。另保留实际 artifact.json 完整字节 SHA256，绑定完整构建闭包。
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

Python 共 73 项（1 项已有环境条件 skip）；包括缺失/失败/非法报告、嵌套原文泄露、
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

## fc8最终ELF依赖误判修复

CI `36970559016` 的前九阶段均通过，包括四项B2真实测试与Android完整链接；
最后ELF门禁错误地要求每个目标都同时需要libc/libdl/libm。实际链接器会移除未使用
的libm，不能为迎合脚本而人为增加无用链接依赖。

依赖规则现为：`libc.so`必须存在，`libdl.so`/`libm.so`可选，任何重复或其他依赖
均拒绝，明确不允许`libMNN.so`、`libc++_shared.so`。原有ELF64/AArch64/PIE、
解释器、LOAD16KiB同余、非WX、GNU_RELRO尾端16KiB及非可执行栈门禁保持不变。
失败时仅报告固定类别：identity/interpreter/dependencies/load_alignment/
writable_executable/relro/stack/malformed_headers，不上传原始readelf路径或内容。

2026-10-02使用现有`android-final.jsonl`的五个实际目标和锁定NDK r30 readelf
重新运行完整parser及证据schema检查，全部通过：

- `build_identity`、`mnn_model_store`：libc/libdl
- `mnn_adapter`、`mnn_executor`、`real_model`：libc/libdl/libm

同时验证两库/三库（及仅libc）合法，缺libc、重复依赖、unknown/libMNN/
libc++_shared拒绝，并验证固定失败类别。此次独立修复不修改native、mobile、
App、receipt方案或Windows workflow；未重编译/执行Android ELF。


## 同次研究凭据本轮验证（2026-10-02 UTC）

冻结实现经独立只读审查未发现 P0/P1 阻塞；审查者另跑 receipt Python 11/11。
本轮 helper 为85 pass/1 skip（actionlint工具未恢复，不把skip计为通过）；Rust34项单元、
4项compile-fail、fmt/clippy通过，含本轮root独立修复的生产cleanup-unconfirmed永久pin回归。
凭据本身只改cfg(test)；不能把“生产无receipt入口”表述成整批变更没有生产修复。

在独立新work中实际执行全部12阶段，逐阶段exit0：七个B1前置、新receipt、四项显式B2真实测试、
Android native、Rust Android、五个最终ELF检查。实际Linux manifest为
`b8b4d8efb06388f49c6457d177997f2bf630c5dceeb9ec190d3cc245a313372d`，
Android为`6ff7a9625cd8bf1135e3f82fa36095da4f4f27ebc0ac808c474020e62e1cb750`。
本地driver SHA为`c5c79ac9e138379a159ce5b2d752ca03cf7904ffed72d0e08b0ab62eaf2f4300`。

随后实际调用`stage_reports`交叉核验，`evidence_verified=true`，missing/invalid均空；
七份proof与上传JSON、receipt原始字节及各自SHA逐个一致。receipt SHA为
`eff52f1647843347110100c704812b900e39406ce2d22c26b2b7da3f262a6e6b`。
因为这是`local-verification`且`source_clean=false`，clean CI总门禁按预期非零、
`all_required_steps_succeeded=false`；不包装成GitHub成功。执行时固定源码范围快照为
`8f618bd1c220095e62182a3ee29f1303fb0eeb8f470fc3f914e630b679049c6a`，后续仅补写本验证文档。
本地证据位于`/workspace/shared/nexa-receipt-complete/complete-verification.json`与`verified-upload/`；
该路径是本次环境中的证据位置，不是发布或可长期访问的产品资产。

本地明确复用已重建native缓存：Linux Makefiles、Android Ninja；每次仍重新configure/build、
postimage审计、export/hash与真实门禁。全新context/bundle/proof隔离，旧失败目录保留。
NDK ZIP此前恢复时已核验，本次复验properties/clang与五个模型文件，并记录恢复证据摘要；
不称本次重新hash已清理的ZIP。容器ptrace限制下本地`detect_leaks=0`，ASan/UBSan保留、LSan未验。
GitHub仍固定Ninja、实际下载/hash ZIP与默认sanitizer门禁，不采用本地复用方式。
正常生产rlib符号及字节检查无receipt/test工厂或相关环境变量入口；normal依赖未变，
registry版本/checksum未升级。Android仅链接和静态检查，设备执行及新提交远端结果仍待验证。
