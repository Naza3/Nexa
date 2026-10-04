# Android verifier B3a 验证（2026-10-02 UTC）

状态：本文保留 `0.1.0+1` 的构建历史，并追加 `0.1.1+2` 取消归因/失焦修复验证。旧版已收到绑定 c0c0927 的真机完整 smoke/safety 与人工后台报告；它们不自动覆盖新版，也不授予生产准入。重置前约27.8MB首包已丢失，旧版历史仅记录恢复后重新运行的证据。

## 0.1.1+2 取消归因与失焦修复

- Flutter inactive不再触发取消；hidden/paused/detached保留冗余取消，Kotlin原生onStop/销毁控制不变，前台恢复不重放
- case只根据实际Executor/adapter观察分类；真实失败优先于并发停止，cleanup-unconfirmed最高优先。无外部停止或对应内部取消证据的原生Cancelled记unexpected_result，不能冒充正常取消
- stop保存首个受控取消原因；随后观察到native-protocol/output-limit故障时升级为首个故障，后来取消不能覆盖。报告封存保留这一原因，不再被通用checkpoint或误造native_failure替换
- 按钮与Dart冗余取消仍共用request_cancelled，未增加来源参数或无界日志；不声称可由报告完整还原触发者
- pubspec版本升级为0.1.1+2；Rust报告从同一pubspec构建读取，prebuilt登记新增其hash；同applicationId与本地既有debug签名。Cargo crate版本及公共native ABI不变
- 默认SAF导出nexa-device-report.txt/text/plain；载荷仍是原始JSON、hash/2MiB/FD/token不变。APK审计检查实际DEX默认名/MIME、manifest版本与源码版本
- 本轮定向测试含普通load/prepare/generate取消、真实故障与取消前后序、无请求取消负例、预期内部取消被外部停止抢先、active_cancel后独立新请求、封存原因/清理优先级，以及实际Flutter observer+FRB控制fixture的失焦/隐藏/恢复；这些是控制测试，不是Android内核/Activity运行证据
- 18项Rust控制测试通过（真实测试默认ignored），clippy/fmt通过；Flutter analyze零问题，7项Dart/observer测试通过。最终真实host闭环56.38秒，17自动passed/6not_run，cleanup confirmed；smoke suite SHA256 `9eb7c6123608c9b7a913649e41d0eb082a526dc64144ef3f1d26653aad6eace2`
- 预提交Android native真实链接9.03秒、APK构建22.7秒及全包审计通过；APK 27,905,682 bytes，SHA256 `6b38eb5f3b3c57b29c2a5b0133845733c00d9405692224fc429efa631b36d063`，绑定3cd8854工作树dirty。此包仅供预提交审计，root提交后必须再clean重建
- 38项prebuilt输入hash、唯一三arm64库/16KiB ZIP与LOAD、bridge严格RELRO/NEEDED、实际默认TXT名/MIME、APK版本与pubspec一致、许可原文字节均通过；Flutter vendor既有RELRO例外保持，不声称每库都有RELRO
- 新旧签名证书SHA256均为 `36025a8400c7dfdd9b87c8ee9b4420e39f199f9ce3ad1edf4a4100905b90c1ce`，versionCode由1升2，同applicationId。覆盖安装/保留模型尚待实际设备确认
- Cargo/Pub锁、FRB生成件和许可正文没有变化；构建前后SDK包manifest清单一致，没有执行adb。验证日志与审计目录位于开发环境`/workspace/shared/nexa-verifier-cancel-fix-precommit/`
- 新版手机失焦/后台/返回不重放和TXT选择器实操待用户复验

## 0.1.0+1 历史预提交构建（以下摘要不用于新版）

### 历史预提交构建身份

- applicationId `io.github.naza3.nexa.verifier`；version `0.1.0+1`；release编译/内部Android Debug签名
- MNN commit `d407447ed56c4121a11ccbd266dc184ca1ead0c2`；patch `dfe571d08b1583e39d7ce271eb289ebc91c06b88261a3c83fdef1e53dc062b80`
- Android native artifact.json SHA256 `6ff7a9625cd8bf1135e3f82fa36095da4f4f27ebc0ac808c474020e62e1cb750`
- 候选digest `1ec59d439451738b4992f2ea5b06438788d752da81d55f11e1e8866d03fa7a57`；直接复用根候选锁，转换来源未知仍明示
- 预提交APK SHA256 `d10d25206f3a82f94b06d787030ca2c4339e9c51c70142d5313e9864eee67a4c`，27,882,858 bytes
- 登记prebuilt.so SHA256 `927f8f4847a628cb9687f29187bba66a358c430019f0759b6be6a1b234bf92e3`
- APK内strip后bridge.so SHA256 `ab98cd529d3ada2c2101fc6418197c49c43a47300470cd4dc5beda8e1e364ae2`；与NDKr30对登记库strip-unneeded所得字节精确相同
- 主notice 4,882,579 bytes，SHA256 `9fc02079a5a04ab144c290c41465c294998efe682024ff7f23d2ec1d4b0306be`；APK资产与源码字节一致

以上是预提交包，不应在root提交后把这些旧摘要写成新source commit产物；最终交付摘要由提交后重建审计文件记录。

## 恢复后的实际命令

所有下列测试/构建成功退出0，失败诊断另列；日志在当前开发环境`/workspace/shared/nexa-verifier-*`，不纳入源码或当作设备运行证据。

- Rust `cargo test --lib --locked --offline`：12单测通过，1真实门禁默认ignored；包括单poll、ack重传/未来ack拒绝、取消后的在途ack、取消唤醒、真实10秒慢消费、设备JSON重复/未知字段、报告2MiB上限/不可变文件、tmp恢复不跟链接、失败case聚合、sticky cleanup控制注入、adapter cleanup_error、并发注册校验不占控制锁/后台往返使发布失效
- Rust `cargo clippy --lib --tests --locked --offline -- -D warnings`、fmt：通过
- 显式真实`real_fd_import_executor_adapter_report_remove`：最终54.70秒通过；固定五文件FD→inbox→公开store→生产resolver精确UnsupportedModel→Executor中英文/system多轮/流式/EOS或stop合并观测/活动取消恢复/卸载close→独立adapter精确预算/超1/真实Stop/load-template-tokenize-prefill-decode跨线程checkpoint取消→报告FD/hash→移除
- 真实链同时覆盖源FD登记后变长拒绝、重复文件名、旧选择token失效、旧report token在另一operation封存后仍读原不可变报告、重复token消费拒绝、同进程open幂等及注入cleanup fault后前后台仍闭门；最后两者明确为控制注入，不伪称真实native销毁故障
- 实际suite报告：17自动case passed、6项目not_run（Activity后台、SAF故障、进程重启、日志canary、stability、core）；后者不包装成Android验收通过
- Flutter analyze：零issue；Flutter test：4项纯Dart回复/UTF-8窗口/重传/边界测试通过
- FRB2.13.0官方重生成，cargo-expand1.0.126；官方generator后按项目edition2024执行cargo fmt规范化，五个生成文件hash与已审源精确不变（generator自身旧edition import排序不同，不把纯排版差异当接口改变）
- Android `tool/prebuild.sh`与真实Flutter release APK构建通过；AGP曾因并行改动的shared test源码使hook报stale并拒绝，冻结输入重新预构建后通过，未放宽hash门禁
- `javap`确认5个Kotlin入口确实是static native且签名与JNI完全一致；JNI/FRB符号共处同一APK桥接库

## APK逐库与manifest审计

`tool/audit_apk.py`核查唯一arm64三库、ZIP所有.so stored且16KiB页对齐、每库LOAD、RELRO实际区间、依赖闭包、strip一致性、37项prebuilt源码输入hash、notice字节、manifest与签名。独立审查复算相同结果。

- `libnexa_device_verifier.so`：AArch64，LOAD16KiB，GNU_RELRO原区间`[0x4be5b0,0x4d8000)`；按16KiB向外取整`[0x4bc000,0x4d8000)`，末端严格16KiB、不覆盖额外RW；NEEDED仅`libm.so/libdl.so/libc.so`
- `libflutter.so`：AArch64，LOAD符合16KiB；RELRO由Android loader按设备页向外取整为`[0xae4000,0xb44000)`，不碰下一RW `[0xb521e8,0xb629e0)`。记录vendor真实行为，不套用自建bridge的裸末端断言
- `libapp.so`：固定Flutter/Dart AOT vendor产物，LOAD64KiB，**没有GNU_RELRO**；没有任何relocation/NEEDED/GOT/PLT，唯一WA section为96B.dynamic和8B.bss。明确记录此无relocation例外，未二进制改flag，绝不声称全部.so都有RELRO
- 无MNN模型大文件、额外ABI、第二份bridge或libc++_shared.so；签名v2校验通过，证书为Android Debug
- min28/target36、研究用途true/生产准入false、allowBackup/fullBackup/cleartext/extractNativeLibs均false；无INTERNET或广泛存储权限；唯一uses-permission是AndroidX自动生成的本应用签名级DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION

## 许可与SDK安装差异

- 原文总表覆盖native库存、Rust普通依赖与标准库、Flutter/Dart/FRB、dart-sys wrapper及SDK、实际Maven releaseRuntimeClasspath
- Maven实际43个去重runtime archive/POM哈希逐个校验，嵌套JAR扫描；唯一内嵌原文是exifinterface1.4.1/LICENSE（保持原字节），无遗漏NOTICE；7份去重原文映射所有实际坐标。Guava继承父POM许可，Flutter两个Maven产物明确绑定engine revision及engine/sky_engine原文
- 依赖锁由官方工具在新环境重新生成，不称为丢失原锁的逐字恢复；mobile运行图共享版本保持一致，额外跨平台锁项不进入Android图
- 恢复后一次AGP构建自动补装官方platform-tools37.0.1，与原先“四SDK包”清单有差异，已报告root并按指示保留；之后显式禁用sdkDownload。没有执行adb/设备操作或接受新的未披露协议；官方repository/archive核对单独留证。不能把旧SDK清单当作未变化

## 旧版构建时未验证条件

以下是旧包构建时的缺测清单；后续旧版真机自报不自动覆盖新版，也不将其余未测项补为通过。SAF provider延迟/撤销授权、Android Activity重建/旋转/进程杀死、屏幕响应、原生后台取消实际时延、设备页大小/驱动/内存温度、无网络长期稳定性与日志canary仍待授权真机验收。普通文件FD的底层I/O和native kernel不可强行抢占；stopping无超时强释放。长期stability按钮禁用，B3b/core及P1生产矩阵未准入。
