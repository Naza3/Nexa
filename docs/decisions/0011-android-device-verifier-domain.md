# ADR0011：独立 Android 设备验证目标与人工证据准入

日期：2026-10-02。状态：已采纳实施边界；APK、Android 真机与生产支持尚未完成。

## 背景

[B2](../t07b-mnn-contract.md) 已提供固定候选 store、MnnExecutor 和共享 core 接缝，但生产 resolver 尚无可信 Android 设备证据，必须拒绝加载。不能为了收集第一份证据，在产品桥接层把 `validated` 改为 true，也不能把已有 Linux `#[cfg(test)] ResearchCpuEvidence` 扩为产品 feature、环境变量或运行开关。

设备证据由独立工程验证目标取得。[P1 日常文本产品](../android-app-parity.md)仍按原门槛交付；可安装的验证 APK 不改变 T08/P1 的完成条件。

## 决策

### 1. 独立目标

- 应用 ID 固定 `io.github.naza3.nexa.verifier`，显示名 **Nexa 设备验证**。优先 debug 研究 APK；若 debug VM 服务无法满足本用途的无网络服务边界，允许 release 编译加内部 debug 签名的研究包，并披露实际 build mode。这不是正式发行或账号签名要求；优化构建性能基线仍另测，不将 debug 吞吐外推到产品。
- Flutter 应用位于 `apps/android-verifier/`；其 `rust/` 是独立 Cargo workspace/lock 的开发 harness，以 path 依赖现有移动三 crate 和共享 core/types。所有这些目录为本决策要求的后续实现，不表示已经存在。
- 不加入根 Windows workspace，不改变生产 `mobile/runtime` resolver，不在依赖 crate 上强开 `cfg(test)`；最终 P1 使用不同 applicationId，不能反向依赖本 harness。
- UI、运行报告及构建身份固定说明 `purpose=android_device_verification`、`research_only=true`、`production_admitted=false`。APK不接收可修改这三项的输入。
- 只有固定候选、CPU profile、受控合成输入和版本化 suite；没有自由配置路径、任意模型执行、通用脚本、后台常驻或公开网络服务。

### 2. B3a 可以独立交付

B3a 通过现有 `MnnModelStore` 校验/导入五文件候选，由同一 snapshot 取得 `resolve_candidate` 与 `MnnExecutor::composition`。先验证生产 resolver 拒绝合法候选，再释放该 resolver；将仍为 `validated=false` 的候选交给公开 `Executor::start` 和 `ExecutionEvents::from_sink`，执行真实 load/generate/cancel/unload/close。

这与现有[直接 Executor 测试](../../mobile/runtime/crates/mnn-executor/src/tests.rs)的底层研究边界一致。harness 仅是固定测试步骤协调者，不实现产品 FIFO、第二套任务调度或自动重放。原生资源仍由既有 MnnExecutor owner 独占。

精确 phase 取消另由 adapter 公开 `Progress` 回调验证；与 Executor 用例串行、资源不并存。Executor 的私有 progress/pressure hook 不进入生产。报告分别标明 `adapter`、`executor`、`app`；B3a 不声明手机上的 core 正向链、队列或 256KiB 账本已经通过。B3b 尚未批准或无设备时，不阻止满足自身门槛的 B3a 工程包交付。

### 3. B3b 必须二次审核

只有 B3a 的精确 Android 报告及关联构建资产经人工审核后，才允许单独提交本 harness 内的私有 `ResearchAndroidEvidence` fixture/resolver：

1. fixture 绑定审核记录与报告 hash、固定模型/模板/策略、CPU 参数、完整 native `BuildIdentity` 六字段及设备/系统/ABI/页大小档；不以商品名覆盖整个芯片家族。
2. 构造器只接受编译入且受审的 fixture；禁止把当前 `build_info` 自动登记为可信，禁止导入报告/manifest自授，禁止由 Dart 提交身份白名单。
3. fixture 成功匹配后，私有 resolver 仅在该研究域内返回 `validated=true`，与同一 snapshot 的原 MnnExecutor 接入原 `Runtime::spawn`。不新增 core 绕过开关，不复制调度器/输出账本。
4. fixture 与 resolver 留在独立验证 crate；生产 `mobile/runtime` 公共 API/符号没有此入口，现有 Linux `ResearchCpuEvidence` 仍只存在于 executor 单测目标。

该步骤取得的是共享 core 的设备研究运行资格，仍不是生产支持。B3b 不在 B3a APK 首次启动、测试结束或报告导出后自动启用。设备自报和报告 hash 不是远程认证或供应链签名。

### 4. 宿主不降低原生安全边界

- Kotlin 原生生命周期必须直接触发 Rust 取消控制，不依赖 Dart 持续运行；停止接收新步骤，等待安全返回再卸载。UI 线程不等待 native kernel，不能强杀线程或超时强释放。
- 句柄和订阅带宿主代际；后台/重建/断流幂等取消，不重放旧生成。旧 owner 未确认退出时禁止新实例；清理未确认保持不可用。
- FRB 使用有界 polling/ack。B3b 保留原 `EventLease` 至消费者确认，不把有界 core 输出抽干到无界 Dart stream。
- SAF 只读五文件，经私有临时 copy 后复用 store 完整校验和原子发布；不把 URI 当路径，不让模型包携带可加载 native 代码。应用数据、模型和导出报告关闭系统备份。
- 报告是只读证据，没有准入/恢复命令。只导出脱敏的固定 schema；无自动上传，无读全机日志或广泛存储权限。

## 从研究结果到 P1

先审核 B3a，再审核 B3b 与所需设备安全/稳定性结果，最后独立变更生产支持矩阵及 resolver。证据必须绑定模型、引擎/补丁/构建、策略、参数和设备；变更身份必须重新评估，不能只改显示名继承支持。OnePlus 15 首测与 Snapdragon 8 Elite 兼容档分别记证，OpenCL/QNN/Hexagon 不继承 CPU 结论。

P1 另实现目录/下载续传、多会话及其余产品要求，并在最终产品 APK 验证宿主/桥接/生命周期与离线行为。诊断组件证据可引用，诊断 APK 的安装成功和截图不能替代产品验收。

## 验证与实施入口

[实施契约](../t07c-android-verifier-plan.md)冻结工位、桥接 DTO、预算、生命周期及分层门禁。构建必须检查生产依赖图/符号无诊断准入，native 与模型身份失配负例拒绝，最终 `.so`/APK 页对齐和许可闭包完整。Apache 修改标记导致 patch 重锁后，旧 artifact fixture 不自动升级。

本决策仅新增独立验证目标与两次人工审核边界；不更改 Windows DTO、IPC、生产模型 schema 或现有 Linux 测试证据。
