# T07-B MNN原生接口与运行时验证

日期：2026-10-02 UTC。B1原生/Rust本地已验证；B2受控store/Executor已实现并完成本地验证，root独立复验通过，远端CI待收口。没有Android设备运行或Nexa APK完成结论。

## B1源码与开发环境

提交 `c651c433e5a3c6cca856bb87cbb2d1bcb3f4fcca`，tree `8c2b4e60d7e286fcf3aa43be92956de5994d2589`，已推送开发分支。根Cargo.toml/Cargo.lock与公共DTO保持原状；同提交另修llama stop前残缺UTF-8静默丢弃，Windows回归单独验证。

实际接口/验证见[原生记录](../../native/mnn-shim/VERIFICATION.md)、[Rust记录](../../mobile/runtime/VERIFICATION.md)、[契约](../t07b-mnn-contract.md)。root独立复跑59项helper含actionlint无skip、native CTest3/3、Rust6unit/4compile-fail、clippy、显式真实模型集成、8项artifact负例、9组privacy与未补丁上游精确对照，均exit0。首次artifact负例调用缺显式环境变量被拒；设置既有verified artifact后全量负例成功，没有放宽门槛。

## B1首轮远端CI：保留失败

[作业36966789118](https://github.com/Naza3/Nexa/actions/runs/36966789118)，job110712168482，整体 **failure**。

- tools、inputs、patch、baseline、linux_native、native_real、rust_linux七阶段success
- Android configure、compile/link、logging audit全部exit0；export阶段exit1，固定错误 `export_archive_missing`
- 根因：CI helper把libunwind定位在sysroot架构目录；锁定NDKr30实际位于clang resource `lib/clang/21/lib/linux/aarch64/libunwind.a`。本地人工构建使用正确路径，不代表CI脚本路径已经验证
- rust_android与最终ELF阶段未运行；完整证据门禁按设计失败，不把已有编译成功写成完整CI通过
- 后续修正用实际clang resource目录并核对NDK边界/版本/AArch64静态成员，补路径及有限缺库名诊断回归；修正后的远端结果待记录

原始私有Actions artifact11211010392，ZIP5041bytes，SHA256 `d3e23a41f835088f343447c86aea653ee718b2bafe83fbeef2e278ee4feab7ad`。下载ZIP、source/tree、七个成功报告及失败报告均按该提交原版schema独立核验；没有invalid report，rust_android/elf明确missing。

### 已完成Linux研究证据

Ubuntu24.04，实际编译器 `g++-13 (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0`，编译器字符串SHA256 `3d4f2be5cf788b5e4ee18bb3b1d5862adb420dc4ae5ec877b5f3d3f2d7923724`。全部native/Rust真实门禁通过；9组privacy、21 prompt/12 completion token精确上游对照。

- 完整artifact.json SHA256：`88c287f25394d6b4565d108592947c1973e7739187ff93e22a45391941450adc`
- shim archive：`a17d2449713ae06c9cf7cbb1ec7f6f7c41dfd67e121bd56daba815bda0e1701e`
- MNN archive：`7efea3ff01012f7e4be351557c7da13b95d2e0cb53c893436e49b455004bebb1`
- MNN commit、patch/policy/header与B1本地锁一致；Rust运行输出再次绑定同一manifest摘要

这组实际证据可供B2 **仅cfg(test)** 的Linux研究factory精确匹配。不能因同一job中Android失败而抹掉已完成Linux证据，也不能反向把Linux通过推广成Android支持。生产resolver不从此测试记录取得validated。

## T07-A回归

同一源码的[原型CI36966789033](https://github.com/Naza3/Nexa/actions/runs/36966789033)成功，artifact11210537029：ZIP9272bytes，SHA256 `edc15ce4e5f45be44816393527605c5a216cc365b7fc6f887b3e302f9ef32412`。全部13份JSON、source/tree及staged schema独立复核通过，all_required_steps_succeeded/evidence_verified/source_clean均true，android_run=false。它是未补丁原型回归，不替代B1失败步骤。

## 后续待验

- 修正后的完整B1/B2 CI；c651 Windows回归已成功，见下节
- B2store/Executor最终真实矩阵、事务故障注入和独立复审
- APK构建与Android目标设备生命周期、取消、内存和日志
- 原生notice文件已准备但修改标记、最终APK资产/入口/链接闭包及Rust/Flutter/模型归属尚待完整收口

## B2本地实现与独立审查

新增独立mnn-model-store/mnn-executor两crate，真实copy→snapshot→core→MNN owner链、预算/多轮/stop/取消恢复及共享256KiB背压已实现。默认测试24单测/4 compile-fail通过；四项真实测试须显式ignored运行，不能把默认跳过称为通过。实现者完整真实矩阵、最终Android五ELF链接和Linux/Android clippy通过；精确命令、时间及限制见[移动验证](../../mobile/runtime/VERIFICATION.md#t07-b2-实现与验证2026-10-02-utc独立于上述-b1-历史记录)。

独立审查发现并推动修复：发布rename成功而父fsync失败后内存未登记，重试可能形成重复generation；直接删除已发布目录的部分失败/中断会破坏重开。现改为发布先登记、失败poison封锁；删除先原子tombstone、受控命名重开恢复。五项新增事务故障注入通过；审查者独立纯store16pass/1真实ignored，并未冒充真实推理测试。

仅cfg(test)研究准入核对精确受审BuildIdentity与固定包身份；生产合法hash仍无Android受信设备证据，必须拒绝。不可恢复native错误安全释放后单发Faulted；既有core事件无法携带最终completion用量，其默认零不代表精确计数。未修改公共DTO以绕过此限制。

原生notice源文件包12组件/27文件已落地，独立hash校验通过；来源/Unicode等价复核见[notice说明](../../native/mnn-shim/notices/README.md)。源库存不代替最终APK链接/许可入口检查。

### root最终独立复验

使用独立Cargo target，最终冻结mobile源码：24普通单测、4 compile-fail文档测试、全targets clippy均exit0；B2三项executor真实门禁合计210.07秒、store真实门禁39.04秒全部通过。CI helper71项（实际actionlint，无skip）、workflow actionlint及notice库存校验通过。修正的Android exporter在锁定NDK/实际build上audit+六库导出exit0，完整manifest SHA仍为 `be2c62705f861ef6115d8a035fd1482662ccda0d11e8fbd642b4ca5b81676a72`。这不是修正后的远端CI结果。

工具链独立模板debug APK也已实际生成并检查，精确身份见[构建锁](../build-lock.md#apk工具链候选安装与独立模板构建已核验)；没有FRB/Rust/MNN，未交付为产品。

## c651 Windows回归已通过

[Windows CI36966789047](https://github.com/Naza3/Nexa/actions/runs/36966789047)、job110712168155全部必需阶段success，包含本轮llama UTF-8/stop修正后的原生、runtime、HTTP/CLI及桌面解压bridge验证。证据artifact11211356112：ZIP77,279bytes，SHA256 `4b2d7a3da2b196abb804f7200542daf1ec6ae63e331d98c4fb68f599b402d4ff`。独立复核50个indexed文件的严格库存/size/hash、源码c651及pass状态；桌面acceptance=result pass、package_unchanged=true、native_window_tested=false。未重新下载或交付桌面产品包，不能扩大原生UI/Windows11/无开发工具/离线或长稳验收范围。B2仅改移动/native notice/Android CI与文档，根Windows代码/锁不变，依据精确paths-ignore不重复触发整套Windows构建。

## B2首轮远端CI：最后ELF规则误判

源码`fc8d87291404ea9b97cb5c5d18b35c0596ab8bc9`、tree`0d1def2e0e1e4f70206946d49840dae6350b0ac2`的[CI36970559016](https://github.com/Naza3/Nexa/actions/runs/36970559016)整体failure，前九阶段全部success：Linux真实B1与四项B2矩阵、Android原生导出及完整五个Rust ELF链接均已通过；仅最终ELF检查失败。

证据artifact11211457947，ZIP7,115bytes，SHA256 `0ae3c19329696af1e44a5fac74b3ed03908d862c2e6434708ef872537b673813`。root按精确fc8脚本/schema/lock/header独立复验九个成功报告及保留的失败报告；source/tree/clean正确，无缺失或invalid report。Ubuntu Linux完整manifest再次为`88c287f25394d6b4565d108592947c1973e7739187ff93e22a45391941450adc`，四个B2真实case均pass，证明本次重建与受审静态测试身份一致。Android完整manifest为`64ad7828607821fa6fca4ca104e786cf1feb4e892e6b00bd04db1b8322dbcb22`，不是本地构建指纹。

根因由root在已存在五个Android ELF独立复现：store与build_identity仅需要libc/libdl，linker合法移除未使用libm；旧helper要求所有文件恰好包含三库而误拒。修正为必须libc、可选libdl/libm、不得重复或包含未知依赖；仍拒绝动态MNN/C++运行库，保留全部ABI、PIE、解释器、LOAD、RELRO、WX及stack门禁。不为满足脚本强行链接无用库。新固定失败类别只披露问题类型，不上传私有路径。

修正后同五个真实ELF parse/schema全部通过，helper73项（root设置实际actionlint，无skip）通过；新的远端结果仍待记录。此次仅提交CI规则与对应说明，不混入正在本地开发的设备验证App或新原生修改标记。


## f4fa90 完整远端门禁与独立验收

源码 `f4fa90abad4e1e58ea785508251026083ae62177`、tree `6866cff3cc9cdc30a392676b07ee0f239c4ac77a` 的 [CI36973082808](https://github.com/Naza3/Nexa/actions/runs/36973082808) 于 2026-10-02 06:48:15 UTC 全部成功，job110731006525。证据 artifact11213655630，ZIP7,784bytes，SHA256 `45200e071f6a137824f280811e82f0f70d5435ef00e87ba7d073901476cd124c`。root 下载后以该提交原版 verifier/schema/lock/header 独立检查全部十阶段报告、source/tree/clean 与四个真实 B2 case；无缺失或 invalid report，all_required_steps_succeeded/evidence_verified=true，android_run=false。

五个实际 Android ELF 全部通过：build_identity/mnn_model_store 依赖 libc/libdl；mnn_adapter/mnn_executor/real_model 依赖 libc/libdl/libm。Android manifest 仍为 `64ad7828607821fa6fca4ca104e786cf1feb4e892e6b00bd04db1b8322dbcb22`。[原型回归36973082807](https://github.com/Naza3/Nexa/actions/runs/36973082807) 同样 success；其产物元数据已读，本节不冒称重新下载复验原型 ZIP。

此结果仅覆盖 f4fa90。随后原生修改标记、新研究证据收据及设备验证 App 是独立修改，必须重新构建验证；不由此取得 Android 真机、GPU/NPU 或生产准入结论。

## 开发环境恢复与当前产物边界

2026-10-02 06:34–06:36 UTC 原执行目录与临时产物不可用；原因未确定。已从远端精确恢复 f4fa90 的 323 blobs/426 tree entries，逐项检查 Git blob SHA 与大小并复原同一 tree/commit，没有创建替代历史。固定工具链、原版 MNN 与五文件模型重新下载并核验；原先未交付的研究 APK 不再存在，不能沿用旧路径或旧构建结论交付。

native 修改标记 patch-set `dfe571d08b1583e39d7ce271eb289ebc91c06b88261a3c83fdef1e53dc062b80` 已在恢复环境重新构建、源审计、原版真实对照和 Android 16KiB 检查，具体证据见 `native/mnn-patches/recovery-build-verification.json`（仓库根相对路径）。App 与收据变更仍在验证；最终 APK、许可证闭包、独立审查和手机安装/生命周期尚未收口。

### 设备验证审查发现的 Executor 清理错误降级

独立审查发现：Prepared::generate 已返回 cleanup_error 时，Executor 原先只设置一般 fault，随后 model.close 成功可把事件降为 Faulted 并释放文件 lease。后续成功不能否定此前未确认的所有权状态。修复为直接 CleanupUnconfirmed，owner 先永久 pin 整组 model/lease 并置 failed；所有后续 cleanup/start/close 保持拒绝，退出尾部不会清除 loaded 或释放已 pin 资源。

root 新增两项定向回归均通过：取消同时携带 cleanup_error 不降级；资源 Drop 不执行且后续 cleanup 不清失败。独立审查复核 owner、退出尾部与 Executor Drop 路径通过。此为生产 Executor 行为修复，不能与同轮仅 cfg(test) 的研究收据隔离说明混淆；完整新真实链与最终 APK 必须重新覆盖。


## 新修改的完整本地收口（GitHub 新提交待验）

原生修改标记、研究收据与 Executor 清理修复的同次本地12阶段全部通过：tools/inputs/patch/baseline/linux_native/native_real/rust_linux/research_receipt/b2_linux/android_native/rust_android/elf。四项B2必须显式运行，Android五ELF实际链接和校验通过。Python86项检查中1项因本地actionlint工具缺失跳过；Rust34单测/4编译失败文档测试与clippy通过。独立收据审查无P0/P1阻塞，并另跑11项收据测试通过。

运行时源码快照SHA256 `8f618bd1c220095e62182a3ee29f1303fb0eeb8f470fc3f914e630b679049c6a`，收据SHA256 `eff52f1647843347110100c704812b900e39406ce2d22c26b2b7da3f262a6e6b`。Linux完整manifest为 `b8b4d8efb06388f49c6457d177997f2bf630c5dceeb9ec190d3cc245a313372d`，Android为 `6ff7a9625cd8bf1135e3f82fa36095da4f4f27ebc0ac808c474020e62e1cb750`。

实际stage_reports得到evidence_verified=true、missing/invalid均空；由于本地有未提交修改，clean CI总门禁按预期拒绝，all_required_steps_succeeded=false。root独立从运行时暂存Git原始字节重算上述源码快照，并按归档模式复核12份报告、七个上传proof原文hash、receipt与B2绑定。随后补写文档使live source context拒绝属于正确行为，不为了更新文档而篡改旧收据或伪造新执行。

本地明确复用已恢复并复验的MNN构建缓存（Linux Makefiles、Android Ninja）；仍重新configure/build/audit/export并运行全部真实门禁。NDK ZIP hash引用同一环境恢复时实际校验，当前重新核查properties/clang/静态成员和模型五文件；不声称再次hash已删除ZIP。ASan/UBSan实际执行，本地LSan因ptrace限制关闭。GitHub仍使用独立新目录、Ninja与实际下载hash，不沿用本地旁路。

设备研究App有独立12单测、4Dart测试、真实host完整链及预提交APK签名/库/对齐/许可审查，见[App记录](../../apps/android-verifier/VERIFICATION.md)。source commit正式绑定须提交后重建，尚无手机执行或GPU/NPU支持结论。
