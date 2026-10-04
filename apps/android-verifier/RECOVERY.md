# 2026-10-02 执行环境重置后的恢复记录

历史快照（07:01 UTC）：当时手写源码已重建但未编译。后续已补齐官方生成件/锁/许可并重新验证，最新事实见VERIFICATION.md；下文保留当时恢复边界，不把旧环境结果冒充新验证。

## 已知事故与历史验证

06:34 UTC 左右执行环境整体重置；repo、共享工具链、原生归档、本地模型、Cargo/Pub/Gradle 缓存、首个研究 APK 和原始日志均不可访问。其他工位与 root 独立确认 overlay 从约29GiB变为约148MiB；不是应用源代码编译错误。本目录不以新建空文件、假锁或旧摘要替代丢失产物。

重置前实际工具返回记载了以下结果，但原始文件已丢失，以下**不能作为当前恢复树的复验结果**：

- `cargo test --lib ... --locked --offline`：6普通单测通过，1真实测试默认 ignored
- 显式 `real_fd_import_executor_adapter_report_remove -- --ignored --test-threads=1`：57.21秒通过；17自动case通过，6人工/后续层次case明确not_run
- `cargo clippy --lib --tests ... -- -D warnings`：06:32 UTC通过
- `flutter test`：4项纯Dart边界测试通过；最后一次analyze有3项括号风格info，源码已修但最终analyze重跑尚未完成
- 最终Android桥接cdylib真实链接，AArch64；LOAD与GNU_RELRO末端16KiB；DT_NEEDED只有libm/libdl/libc；JNI与FRB共用单一库名
- 首包 `flutter build apk --release --target-platform android-arm64 --no-pub` 122.2秒成功，约27.8MB；应用ID/minSDK正确，包含且只包含arm64的libapp.so/libflutter.so/libnexa_device_verifier.so；无INTERNET；AndroidX生成本应用签名级dynamic receiver权限
- 首包APK v2签名验证与`zipalign -c -P 16 -v 4`通过，签名为内部Android Debug；并无真机安装/SAF/后台运行证据

首包之后又做了源码格式化和未完成的最终审查。不能把历史首包当作最后源码构建产物。

## 恢复范围与保真边界

- 手写Rust API、host、sink、runner、JNI；Dart facade/UI/4测试；Kotlin SAF/JNI控制；受控NativeAssets hook；显式Cargo预构建/hash登记/许可收集脚本；手写Gradle与manifest已恢复
- 恢复依据为本turn留存的完整工具创建文本、已成功的增量替换和工具输出。源码注释/排版经手工归并，不能声称与重置前最后格式化文件逐字节相同；需重新codegen/fmt/test/build/review
- `android/.gitignore`在恢复时不再忽略Gradle wrapper，以便后续把从Gradle9.3.1重新生成的wrapper纳入源码；这是原先已计划但重置前未执行的小修正
- 许可收集器在恢复时明确把缺dart-sys wrapper许可标为missing，避免仅Dart SDK许可证替代wrapper原文；原版此处仍待补
- 所有其他待改代码问题仍列下方，不凭恢复过程声称解决

## 有意不伪造的缺失输入

1. `rust/Cargo.lock`、`pubspec.lock`：旧锁丢失，须从精确直接版本重解并和根/mobile锁复核共享依赖；不能把新锁称为原锁
2. FRB2.13.0生成件：`rust/src/frb_generated.rs`、`lib/src/rust/**`及可选C头；必须重新运行精确官方generator，不手写mock
3. Flutter3.47.6模板生成资源、`android/build.gradle.kts`、Gradle9.3.1 wrapper脚本/JAR、图标/launch styles、`.metadata`：从官方固定版本在独立临时模板恢复，不猜测二进制内容
4. `assets/THIRD_PARTY_NOTICES.txt`、notice inventory及原文：依据实际恢复后的Cargo/Pub/Maven/native闭包重新生成。已核实的补充原文来源在`assets/upstream/SOURCES.json`，没有空license占位
5. `build-input/native.json`、native `.so/.a`、link map、APK、签名文件、模型、SDK或任何旧hash日志：必须重建/重新验签与审计，不提交这些产物

## 审查后待办（恢复前已知，不能跳过）

- 增加纯控制回归：中途case失败不能总passed；异常后case journal保留已跑/未跑；CleanupUnconfirmed不能被Unload.start失败覆盖；清理不确认后同进程open/前后台/重复启动仍永久闭门；尚未写入这些最后回归
- 复审adapter所有Prepared/GenerationFailure.cleanup_error错误分支，确保不能借Drop忽略销毁错误；不确认必须pin模型及lease。不安全注入真native时仅称控制测试
- 再验成功末尾输出的poll/ack排空、取消后的最后合法ack、10秒慢消费/断流与生命周期门禁；stop设置不等于kernel返回
- suite hash已覆盖完整runner源及suite ID；顶层profile是baseline，每个变体必须继续核对真实requested/effective、不能把8/16/32 token变体记成256
- build.source_dirty当前是字符串，需改bool/null并对未知字段给准确缺测原因
- SAF流程Activity重建/重复结果/后台取消、报告导出源FD每失败分支关闭，需要Android侧单元/设备验证；当前MainActivity错误只映射受控code
- 许可：dart-sys精确commit原文；Flutter/Dart/Rust文本去重；实际Android Maven releaseRuntimeClasspath的许可/NOTICE闭包还未审完；不得以脚本missing为空冒充全部APK许可闭包
- 完整FRB重生成无差异、Rust fmt/clippy/unit/真实host链、Dart analyze/test、Android cdylib与最终APK全部.so/RELRO/ZIP/signature/manifest/notices复验后再冻结源码
- B3a safety含人工未测时应inconclusive；长期stability禁用；B3b/core/生产准入均未实现或未运行，保持research_only=true、production_admitted=false

没有手机连接，不宣称真机首启、生命周期时延、离线长期稳定或设备通过。
