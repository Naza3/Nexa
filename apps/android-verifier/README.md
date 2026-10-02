# Nexa 设备验证（B3a 研究包）

独立 applicationId `io.github.naza3.nexa.verifier`，显示名“Nexa 设备验证”。`research_only=true`、`production_admitted=false`。固定五文件导入 → 公开store/Executor/sink真实CPU运行 → 独立adapter检查 → 取消/卸载 → 本地脱敏报告。生产resolver始终拒绝，没有修改validated、产品FIFO、聊天数据库、网络下载或GPU开关。

当前工程已在恢复后的环境完成Rust/Dart测试、真实Linux host闭环与Android APK静态审计；**没有Android手机安装/运行证据**。最新验证与剩余门槛见 [VERIFICATION.md](VERIFICATION.md)；07:01的源码恢复历史独立保留在[RECOVERY.md](RECOVERY.md)，不代表当前仍缺生成件。

## 固定构建输入

- Flutter3.47.6/Dart3.13.5、Rust1.98.1、FRB Dart/Rust/codegen2.13.0、cargo-expand1.0.126
- JDK17、Gradle9.3.1、AGP9.1.0、Kotlin2.4.0、compile/targetSdk36、minSdk28、NDKr30/API28、arm64
- 独立Cargo.lock/Pub锁；只使用仓库固定MNN patch/artifact和候选模型锁。Android模型五文件不嵌入APK
- release编译、内部Android Debug密钥签名；不是正式发行签名或生产准入
- `android.builder.sdkDownload=false`，禁止AGP继续自动增加SDK包；`org.gradle.daemon=false`避免跨构建保留环境代理

## 生成、测试与构建

先准备符合`mobile/runtime/README.md`的真实native artifact；缺失或身份不匹配直接失败。本文命令在App目录执行。环境变量必须来自当前可信构建环境，不把开发机绝对路径写入源码。

```sh
# 精确安装/准备上述工具，先取齐依赖；通常只在锁变化时联网。
flutter pub get
export NEXA_MNN_ARTIFACT_DIR=/absolute/audited/linux/artifact
export CARGO_TARGET_DIR=/absolute/verifier-target
export CARGO_BUILD_JOBS=2
flutter_rust_bridge_codegen generate --no-deps-check --no-dart-format --no-dart-fix
cargo fmt --manifest-path rust/Cargo.toml # 规范化generator的旧edition import排版
cargo fmt --manifest-path rust/Cargo.toml --check
cargo test --lib --manifest-path rust/Cargo.toml --locked --offline
cargo clippy --lib --tests --manifest-path rust/Cargo.toml --locked --offline -- -D warnings
flutter analyze
flutter test

# 显式真实Linux host门禁；会在私有临时目录双copy模型并实际运行。
export NEXA_MNN_TEST_MODEL=/absolute/fixed-five-file-candidate
cargo test --lib --manifest-path rust/Cargo.toml --locked --offline \
  real_fd_import_executor_adapter_report_remove -- --ignored --test-threads=1

# Android唯一代码资产：显式Cargo离线预构建，hook只核hash并登记。
export ANDROID_NDK_HOME=/absolute/android-ndk-r30
export NEXA_MNN_ARTIFACT_DIR=/absolute/audited/android/artifact
./tool/prebuild.sh
flutter build apk --release --target-platform android-arm64 --no-pub
python3 tool/audit_apk.py build/app/outputs/flutter-apk/app-release.apk /absolute/audit-output
```

Linux原生静态归档没有PIC，`cargo test --lib`用于真实host测试；不声称Linux cdylib可链接。Android cdylib是实际完整链接。生成件/锁变化须单独复核；禁止编辑报告字段来伪装提交或构建身份。提交后重新prebuild才能让报告绑定新source commit与clean状态。

`tool/register_prebuilt.py`记录最终.so及全部相关Rust/Cargo/锁/头文件hash；NativeAssets hook拒绝过时输入，不自动调用另一个Cargo、下载引擎或回退mock。JNI `System.loadLibrary`和FRB显式加载同一个`libnexa_device_verifier.so`，最终APK只有一份此库。Gradle strip后的实际库须与NDKr30对登记库进行同一strip所得字节完全相同。

许可生成：用`tool/runtime_inventory.gradle`取得实际arm64 releaseRuntimeClasspath，再由`package_maven_notices.py`校验archive/POM/内嵌notice及原文；`package_notices.py`汇总native/Rust/Dart/Maven原文。最终主notice字节必须和APK资产一致，UI有独立许可入口。

## 运行与安全边界

- SAF只选择系统允许读取的五个普通、只读、可定位文件；不跨读其他App私有沙箱，不申请持久SAF授权或广泛存储权限
- Rust在锁外dup/校验FD，发布token再次检查前台和lifecycle sequence；分块复制到0700私有inbox，每个固定长度重新核验，再由store二次copy/hash/原子发布
- 单operation，无产品等待队列；每次最多一个poll、一个待确认事件和一个待发送槽。4KiB文本/16KiB事件、10秒无消费进展取消；UI只保留最近64KiB正文
- Kotlin直接JNI请求后台取消，独立于Dart和provider I/O；取消不是安全结束证明。未返回的内核持续stopping；CleanupUnconfirmed永久闭门，保留模型/lease，不超时强释放
- 前台恢复、重建或进程重启不重放旧请求。进程中断留下interrupted/process_ended_unknown记录，不补造Unloaded
- 每份报告≤2MiB、不可变UUID文件和限域一次性只读token；导出只含受控字段、计数/hash，不含正文、URI、用户路径、序列号或完整fingerprint

`b3a_smoke_v1`有17个自动case；`b3a_safety_v1`含人工未测项时为inconclusive；长期stability明确禁用。设备SAF/Activity/provider/进程杀死/日志canary/热内存与长期稳定门槛、B3b/core及生产准入仍待后续授权验证。独立手机操作步骤见[设备验收指引](../../docs/android-device-verifier-acceptance.md)。
