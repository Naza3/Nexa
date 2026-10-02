# T07-C 前置：Android 设备验证 APK 实施契约

日期：2026-10-02。状态：已批准实施边界，尚未实现 APK。依据[ADR0011](decisions/0011-android-device-verifier-domain.md)、[B2 接缝](t07b-mnn-contract.md)和[产品分期](android-app-parity.md)。B2 源码基线 `fc8d87291404ea9b97cb5c5d18b35c0596ab8bc9`；[新 CI36970559016](https://github.com/Naza3/Nexa/actions/runs/36970559016)在编写时进行中，不预记通过。

## 1. 固定范围与唯一写入工位

交付 `io.github.naza3.nexa.verifier` / **Nexa 设备验证** 的 arm64 研究 APK，优先 debug 编译，必要时按第6节改 release 编译加内部 debug 签名。先做 B3a 真正 adapter/executor/app 闭环；B3b 必须在设备报告人工审核后另行受审。无自由聊天、下载器、多会话、GPU/NPU 开关、模型自动准入、网络服务或自动上传。

| 工位 | 独占写入范围 | 交付责任 |
| --- | --- | --- |
| Rust/bridge | `apps/android-verifier/rust/**`、`apps/android-verifier/lib/src/rust/**`、`apps/android-verifier/flutter_rust_bridge.yaml` | 独立 Cargo workspace/lock、固定 suite、store/Executor 组合、FRB API/生成文件、native 取消入口、报告与边界测试 |
| UI/Android 壳 | `apps/android-verifier/`其余文件，含`lib`手写代码、`android`、`hook`、Pub 锁、Flutter 测试 | 原生 SAF/生命周期/设备采样/导出，页面、poll/ack 消费，Native Assets 集成 |
| root 整合 | `scripts/android_verifier/**`、相关 CI 与验证/索引/状态文档 | 身份/依赖/许可/最终 APK 检查、构建协调、人工审核记录、精确提交发布 |

上述路径均为后续实现范围，不是现成命令。生成 FRB 文件只由 Rust 工位运行；Pub/Cargo 锁不由两工位同时改。原有 `mobile/runtime`、native、共享 core/types 和 Windows 代码不归本片改动范围；缺接口先报告，不暗加生产研究开关。父级整合提交，子工位不派生、不提交。

## 2. 固定输入与真实执行

- MNN commit `d407447ed56c4121a11ccbd266dc184ca1ead0c2`；本片须使用补齐修改标记后的 patch `dfe571d08b1583e39d7ce271eb289ebc91c06b88261a3c83fdef1e53dc062b80`。对应新 native artifact 正在重编，实际 policy/artifact 指纹需构建完成后核验；`fc8d872` 的 B2 CI 仍按旧身份独立留证，不能混用到新 APK。
- 五文件直接复用[candidate-model.json](../scripts/android_mnn/candidate-model.json)，来源 revision `34dfccda1187ded6e07ea06426da576b0b793c6b`；模型 ID `qwen3-0.6b-mnn`、包 digest `1ec59d439451738b4992f2ea5b06438788d752da81d55f11e1e8866d03fa7a57`。不复制第二份可独立漂移的模型锁。
- 基线 `cpu / context=2048 / threads=2 / batch_size=32 / max_tokens=256 / temperature=0 / top_p=1 / seed=0 / nonthinking`。个别边界 case 的输入/参数编译入版本化 suite 并进入其 hash；不得透传 UI 任意参数，不能沿用 core 默认 batch512。
- `b3a_smoke_v1` 为安装后首测，`b3a_safety_v1` 为取消/负例/生命周期，`b3a_stability_v1` 为显式长测。B3a 不公开 B3b suite；不认识的 ID 明确拒绝。

Executor 用例唯一链路：`store.snapshot()` → `resolve_candidate` → `MnnExecutor::composition` → 断言生产 resolver 拒绝 → 丢弃 resolver → `Executor::start(Load/Generate/Unload, ExecutionEvents::from_sink)` → `close`。候选始终 `validated=false`，完整 load 重 hash 和受控 config 继续由原 store/owner 执行。实际 API 见[store](../mobile/runtime/crates/mnn-model-store/src/lib.rs)、[executor](../mobile/runtime/crates/mnn-executor/src/lib.rs)与[Executor 契约](../crates/runtime-core/src/executor.rs)。

runner 只协调一个固定 suite，重复启动返回 busy，不维护产品等待队列。adapter phase 用例须在 Executor close 后才创建自己的模型；其 Model/Prepared/lease始终同线程并按序释放。报告层次必须如实区分，不能由 adapter phase 通过推导 Executor/core 同 phase 通过。

## 3. 桥接 V1 与句柄契约

以下是待实现的固定 API，不是当前已存在的函数。FRB Rust 入口使用 snake_case，Dart 生成名称以生成器输出为准，手写 UI 只经一个 facade 调用。DTO 使用受控 enum/结构；如序列化为 JSON，拒绝未知字段/重复 key/非有限数。不得暴露 Model、Prepared、任意路径/URI、日志正文、外部配置或任意命令。

公共值：`HostEpoch` 为进程内随机不透明代际，`OperationId/SelectionToken/ReportToken` 为不透明一次性/限域 ID；桥接编码均为固定格式 UUID 字符串。`Sequence` 为单 operation 从1递增的 u64。所有非初始化操作携带 epoch，旧 epoch 返回 `stale_handle`，不触碰新任务。每次进程启动新 epoch，不能从磁盘恢复 native 句柄；`verifier_open`进程内幂等，不能因页面重建清空或另建活跃host。

| FRB API | 固定请求 / 返回 |
| --- | --- |
| `verifier_open()` | 无参数 → `VerifierSnapshot`；绑定原生壳已登记的可信 App 私有目录和设备信息，Dart 不提供目录 |
| `verifier_snapshot(epoch)` | → `VerifierSnapshot`，只读取缓存状态，不做大文件 hash 或等待 kernel |
| `candidate_import(epoch, selection_token)` | → `OperationRef`；原子占用唯一工作槽后消费原生选择 token；忙碌拒绝不消费 |
| `suite_start(epoch, suite_id)` | → `OperationRef`；固定候选就绪且前台才受理，立即登记 operation 后返回 |
| `operation_next(epoch, operation_id, ack_sequence?)` | → `PollReply`；单消费者，最多1秒心跳，按第4节确认与预算 |
| `operation_cancel(epoch, operation_id)` | → `{ operation_id, state: stopping }`；只确认发出取消，安全终结另取 |
| `candidate_remove(epoch)` | → `OperationRef`；只在无测试/导入活动且清理已确认时开始，先释放所有 snapshot/owner再调用 store删除 |
| `report_prepare(epoch, operation_id)` | → `ReportDescriptor`；只读取已封存报告，返回 opaque token/hash/字节数，无私有绝对路径 |

`VerifierSnapshot` 固定字段：`schema_version=1, host_epoch, purpose, research_only, production_admitted, build, device, candidate, host_state, active_operation, latest_terminal, last_error`。三个用途/准入字段按ADR固定；`candidate={model_id, artifact_digest, state: absent|importing|ready|durability_unconfirmed}`；`host_state=idle|importing|running|stopping|cleanup_unconfirmed|unavailable`。`build/device`按第7节白名单，未知项为null且附原因，不填0冒充测量。

`OperationRef={operation_id, kind: import|suite|remove, state: running|stopping}`；`PollReply={operation_id, event: EventEnvelope|null, terminal: TerminalSummary|null}`，null event是心跳，无新sequence。删除期间snapshot的host_state为running；candidate有无以实际store注册状态为准，不预报已删除。

`EventEnvelope={sequence, kind, payload}`为tagged enum，payload固定：`progress={stage, completed_bytes?, total_bytes?}`，stage仅`source_copy|store_import|loading|preparing|generating|stopping|unloading|removing`；`case_started={case_id, layer}`；`text_delta={text}`；`case_result={case_id, layer, verdict, reason_code?, duration_ms?}`；`terminal=TerminalSummary`。phase细分及完整指标留报告，Executor无法观测的native phase不得伪填；字节进度不可得时使用null。

`TerminalSummary={operation_id, outcome: passed|failed|cancelled|interrupted|inconclusive, cleanup: confirmed|unconfirmed|process_ended_unknown, report_id?, error?}`。`VerifierError={code, retry: allowed|reopen_required|process_restart_required|not_applicable}`；code为审定枚举，不返回任意原始错误字符串。保留现有 runtime 错误码；本桥接新增 `stale_handle, busy, invalid_suite, invalid_ack, invalid_native_input, unsupported_source, selection_expired, selection_consumed, space_insufficient, report_unavailable`；B3b才增加`unsupported_device_fixture`。同一次 operation只封存一次终态；重复读取返回同一小型摘要，不重复正文。

## 4. 输出预算、确认与取消

- 每 operation只允许一个未完成poll、一个未确认EventEnvelope；text_delta最大4096 UTF-8 bytes，完整单事件编码最大16KiB。suite事件总数受编译入计划上限约束，不能无限累积日志。
- `ack_sequence`只确认上一已交付事件。相同确认重试幂等，确认未来/其他operation序号返回invalid_ack；未确认时重新poll只能重发同一事件，不提取后续事件。Dart按operation/sequence去重。
- B3a sink最多一个待发送文本槽加一个在途事件；有界控制摘要单独保留，进度可合并，终态不可被满文本槽挤掉。sink等待时必须同时检查取消/断开，并由控制路径唤醒；10秒无实际消费进展触发slow_consumer取消。重复请求同一事件不算进展。
- B3b把原 `EventLease`保留到ack；不使用`recv()`提前归还信用，不增加另一个256KiB账本。FRB序列化的常数份复制另计入桥接上限，不能宣称零复制。依据[现有lease](../crates/runtime-core/src/output.rs)。
- UI展示窗口最多64KiB UTF-8正文，超出移除旧片段并显式显示“仅保留近期输出”；报告只累计长度/hash等指标。后台/解绑清掉显示正文；不把它写聊天库。
- 取消、后台、poll断流不经过suite运行锁或输出槽；先关闭新步骤入口、唤醒sink、设置当前CancellationHandle/adapter atomic。取消不等待下一次poll，不能排在generate后面。终态失败/取消可丢弃未确认正文，保留安全终态摘要；清理确认前不启动下一模型。

## 5. 原生壳接口、导入与生命周期

Android壳的 PlatformChannel 固定为 `io.github.naza3.nexa.verifier/platform_v1`：`pick_candidate({epoch})→SelectionToken|null`、`cancel_selection({epoch,selection_token})→null`、`export_report({epoch,report_token})→saved|cancelled`、`show_licenses()→null`。错误转为同一VerifierError；选择/导出取消不改变模型或测试结果。应用不申请广泛存储、READ_LOGS、摄像头、麦克风或通知权限；不申请SAF持久访问，首片不跨重启继续外部源读取。

仅原生壳使用的 Kotlin 对象固定为 `io.github.naza3.nexa.verifier.NativeVerifier`，以下方法均为`@JvmStatic external`，Rust工位实现对应静态JNI符号（第二参数为jclass）。除JNI系统自身不可恢复异常外，方法返回UTF-8 JSON字符串，固定`{"ok":true,"value":...}`或`{"ok":false,"error":VerifierError}`，不抛业务异常；输入最大16KiB、返回最大16KiB，严格拒绝未知字段与非法长度。Dart不能经PlatformChannel传这些原生参数。

| Kotlin静态方法签名 | value / 处理职责 |
| --- | --- |
| `nativeBootstrap(canonicalAppRoot: String, deviceJson: String): String` | `{host_epoch}`；Kotlin从App context建立专用0700私有root并canonicalize，Rust再次核验；相同root重复调用返回同一epoch，不重置host，不接受运行中更换root |
| `nativeRegisterCandidate(epoch: String, names: Array<String>, lengths: LongArray, readFds: IntArray): String` | `{selection_token}`；三个数组各恰好5项，按名称对应候选锁；Rust核对长度/只读/普通可定位FD后dup，各失败分支关闭已dup副本 |
| `nativeCancelSelection(epoch: String, selectionToken: String): String` | `{state: released|consumed}`；未消费token释放全部副本，已释放重复调用幂等；已消费返回consumed，导入只能由operation_cancel取消 |
| `nativeVisibility(epoch: String, visible: Boolean, lifecycleSequence: Long): String` | `StopReceipt`；序号为0..i64::MAX，单调去旧；设置门禁/取消/唤醒后快速返回，不等待kernel |
| `nativeOpenReport(epoch: String, reportToken: String): String` | `{fd: Int, size_bytes: Long, sha256: String}`；成功时消费token并把只读FD所有权交Kotlin，FD≥0；失败不交FD，不提供任意文件入口 |

`deviceJson`固定为`{schema_version:1, manufacturer:String, model:String, soc_manufacturer:String|null, soc_model:String|null, android_release:String, sdk_int:Int, security_patch:String|null, supported_abis:String[]}`。单字符串最大128 UTF-8 bytes，ABI最多8项且每项32bytes，sdk_int为28..1000；缺失信息为null（manufacturer/model未知用固定unknown）。禁止添加序列号等字段；实际页大小由Rust在同一进程sysconf取得，不信任Dart或Kotlin猜值。内存/热采样暂不可得可在报告写not_run，不为第一片扩展此初始化DTO。

SAF打开由Kotlin后台线程及CancellationSignal处理。注册调用期间Kotlin保有五个原ParcelFileDescriptor，Rust只持dup副本；Kotlin在finally中关闭自己的原FD，成功失败均如此。每host最多一个未消费selection，有效期10分钟；活动操作期间登记返回busy，空闲时登记新选择先废弃旧未消费token，后台或显式cancel_selection同样释放。picker结果在Activity恢复后登记，避免该picker自身的后台通知使新token失效。`candidate_import`取得工作槽后原子消费token，把副本所有权转给Rust导入worker，过期返回selection_expired；busy不消费，已消费重交返回selection_consumed。worker负责FD→私有inbox→store的分块copy/hash，取消只置标志，worker退出finally才关闭其FD，不能从别的线程乱关正在使用的FD。Dart不搬字节，Kotlin不执行第二套模型copy。

首片只接受可校验为普通、可定位且长度正确的只读FD，使用受控偏移读取；管道/不可定位provider返回unsupported_source，不尝试无限阻塞读取。即便普通FD的底层provider返回慢，也只可等待安全返回，不保证源I/O立即可中断。report导出先让用户选保存目的地，取消则不打开源报告FD；选定后Kotlin同步取得nativeOpenReport结果、包装并在finally关闭源和目的FD，按长度/hash校验copy，不修改封存报告。需要重试时重新report_prepare；未消费report token限一个且10分钟过期，可跨系统保存对话框导致的后台事件存活，因为它只读已封存文件，不授予运行/模型访问。

`StopReceipt={host_epoch, lifecycle_sequence, state: idle|stopping|cleanup_unconfirmed|unavailable}`。原生返回只代表门禁/取消已设置，不代表卸载完成；重复通知幂等，Kotlin不持有native模型指针。初始化默认为不可提交，收到匹配epoch的visible=true后才开放显式start。JNI与FRB必须绑定同一个被Native Assets打包/加载的cdylib实例，不能另打第二份native库或建立第二套Rust全局状态。

### SAF五文件copy

1. 显式选择候选目录或五个文件；严格要求锁中的五个名称和长度，不接纳额外项、重复名、子目录、archive或native库。ContentResolver只读源，已获token对应的句柄只用于该次导入；不解析content URI为磁盘路径。
2. 受理导入前检查App文件系统可用空间。固定包 `S=454470710` bytes；私有inbox加store staging最低 `2*S`，预留 `128MiB`，因此本片 `required_free_bytes=1043159148`。预检不是ENOSPC保证，实际写入/fsync错误仍安全终结。已有包不叠加导入第二份。
3. 在store根之外创建应用拥有的0700受控inbox，文件0600，分块读取且核对实际长度上限；每块检查取消。不得将整个权重读入Dart/内存。外部授权失效、长度变化、磁盘不足均不得发布模型。
4. 五文件临时copy完成后，以私有路径调用现有 `import_candidate(source, cancel)`，由store再次copy/hash、严格metadata/引用校验、fsync和原子发布。成功或确认失败后删除自有inbox；store poison时显示durability_unconfirmed，释放旧句柄后重开核验，不盲目重复导入。
5. 重启仅清理本App严格命名的私有残留，不删除外部源、不跟随链接、不猜测清理未知目录。数据根从可信App context取得canonical路径，避免`/data/data`别名祖先触发store拒绝。模型/报告不放可被系统随时清理的cache；备份关闭。

### 生命周期与所有权

- Kotlin在应用不可见时直接进入native控制入口；Dart生命周期只作冗余通知。原生入口快速返回，重hash/native返回/卸载在后台工作线程完成。旋转/Activity或Flutter重建可保守取消，但不能创建第二owner或复用旧订阅。
- B3a停止当前suite/导入，取消活动调用；确认其终结后再发Unload并close。B3b使用原`RuntimeHandle::shutdown`，复用取消队列/活动load/generate→安全卸载→close，不在Dart重写此调度。
- `stopping`期间所有新导入/测试/删除拒绝。无kernel返回时持续stopping；明确CleanupUnconfirmed则永久不可用，不能超时强释放或重新加载。前台恢复不自动重放、不自动开始下一case。
- unload后snapshot仍固定generation；删除前必须关闭owner/runtime并释放所有snapshot/resolver，再调用`remove_generation`。导入/删除后取得新snapshot，不假定旧composition自动看到新注册表。
- 每run先持久化小型“开始”记录，结束原子封存终态；系统终止进程后重开将未封存run记为interrupted/cleanup=process_ended_unknown，不补造Cancelled/Unloaded事件，不恢复请求。新进程依store既有恢复规则重新核验。

## 6. 构建与可独立交付门禁

Flutter3.47.6/Dart3.13.5、JDK17、Gradle9.3.1、AGP9.1.0、Kotlin2.4.0、compile/targetSdk36、minSdk28、NDKr30/API28/arm64为已完成官方模板APK烟测的组合；该结果不覆盖FRB/Rust/MNN。本片FRB Dart/Rust/codegen/hooks选定2.13.0，首次真实集成通过后才记为Nexa已验证锁。

采用[FRB Native Assets](https://cjycode.com/flutter_rust_bridge/manual/integrate/native-assets)单一打包路径，Rust crate声明staticlib/cdylib并固定Rust1.98.1。先审查精确版本hook的Cargo参数和输入追踪：`--locked`、已取齐依赖后的`--offline`、Android linker/API、显式native artifact目录、最终cdylib的两个16KiB link参数，以及独立target目录必须受控。若默认hook不能满足，使用显式Cargo预构建加本地hash核验的code asset登记；不能默默切换工具链、下载MNN/模型或回退假实现。

构建实施按顺序留下命令/退出码和证据，实际脚本实现后再在README发布调用方法：

1. 固定新patch/notice身份，跑原native/adapter/store/executor适用回归；核对本片精确源码CI，不能用旧提交成功替代。
2. Rust fmt/clippy/unit、Flutter analyze/unit、FRB生成差异检查；共享依赖版本与两个现有lock比对。缺native输入必须失败。
3. 完整Android cdylib链接，检查native静态归档成员、link map、DT_NEEDED和唯一C++闭包；禁止重复打包另一份MNN/C++实现。检查root Windows依赖图无MNN、生产mobile rlib无诊断准入/压力hook。
4. 真正Flutter研究APK；提取检查全部.so（含Flutter）的ABI/LOAD/RELRO、APK ZIP 16KiB对齐、签名、applicationId/minSdk、用途标记、网络/备份/权限manifest、notice文件hash与可访问许可入口。模板APK或五个测试ELF不替代此步。
5. 在获授权的实际设备安装/启动/导入/真实生成/取消/卸载并导出报告。没有设备时只交“工程包，设备待验证”，不能标B3a设备通过。

先完成APK集成包，再收集B3a真机结果；不等待B3b或P1。debug Flutter工具可能引入VM服务/INTERNET权限，必须实查并披露，不得凭空白模板通过断言本用途包无服务。若debug无法满足无网络服务边界，允许改用release编译加仅内部debug签名的研究包，重新检查最终APK并记录build_mode/signing_kind；仍保留独立applicationId/research_only，不要求正式发行或账号签名。优化构建性能基线另跑，不能因使用release模式自动宣布性能验收。模型不嵌入APK，也不把约433.4MiB模型或编译产物提交源码。

## 7. 脱敏只读报告 V1

每operation报告为有大小上限的UTF-8 JSON（最大2MiB）；case数、样本次数及错误码均由suite限制，满额明确失败而非静默截断。UI不编辑报告，导出不会改变结果、模型manifest或准入。导出内容仅白名单字段：

- `schema_version=1, purpose, research_only=true, production_admitted=false, report_id, operation_id, suite_id, suite_sha256, started_at_utc, finished_at_utc, terminal`；import/remove报告的suite_id/suite_sha256为null，不伪造测试suite
- `build`：源码commit/是否dirty、APK版本/build mode/applicationId、APK与最终桥接.so的实测hash、FRB/Flutter/Rust/NDK版本、原`BuildIdentity`六字段；无法获取者null+原因，发布资产hash由外部复核对照，不编造自包含hash
- `model`：固定model_id、五文件/包/模板身份、来源revision、已知/未知转换来源；`profile`为本次实际CPU参数，requested/effective层次明确
- `device`：manufacturer/model、可用SoC信息、Android版本/API/安全补丁、ABI、实际页大小；可用的内存/热状态采样及来源。无序列号、Android ID、账号、IP、完整build fingerprint或用户文件路径
- `cases`：固定case_id、layer=`adapter|executor|core|app`、verdict=`passed|failed|not_run|inconclusive`、受控reason_code；prompt/completion计数、输出字节数/hash、阶段耗时、取消请求至安全返回/卸载的分开样本、内存/热状态及缺测原因。不导出输入/输出正文、URI、异常堆栈或原始logcat
- `coverage`：明确哪些层/phase实际观察，哪些仅交叉链接/合成压力，哪些未运行。已知fatal Faulted缺最终completion usage的路径填unknown，不能把默认0当精确值

`report_prepare`返回`{report_token, report_id, schema_version, sha256, size_bytes}`；原生壳只能将这份封存文件复制至用户显式选择的目的地。hash用于完整性对照，不是可信签名。日志canary以外部获授权的受限设备采集独立验证；App不申请READ_LOGS，缺采集时报告not_run。设备/报告自报信息不自动成为支持证据。

## 8. 分层验收与人工审批

| 门槛 | 必须留证 | 不得推导 |
| --- | --- | --- |
| B3a smoke | 实际ABI/build身份、正确包import、生产resolver拒绝、真实中英文/system多轮、精确预算/超1、stop、取消后新请求、卸载/close再加载 | 不代表core或P1完成 |
| B3a safety | adapter公开phase跨线程取消；Executor真实活动取消；缺/坏文件、SAF取消/授权失效、copy/空间失败；UI断流/重复操作；后台原生取消与卸载、重建/进程重启；日志canary分报 | 不将检查点样本冒充任意时刻1秒保证；不将注入ENOSPC写成真实满盘 |
| B3a stability | 显式运行100次短请求、20次load/unload、15分钟持续生成，记录温度/内存趋势、UI响应与实际释放 | debug性能不是优化包性能；B3a层次不冒充core A19/A23全通过 |
| B3b研究域 | 人工审定B3a报告后新增私有fixture；原core单活动/一个等待槽、单终态、预算/慢消费/断流/取消/安全shutdown及FRB lease/ack真机回归 | 不向生产注册表自动写入；精确phase不可观测时保留缺口 |
| P1准入 | 对应设备必要A19/A21–A23/A26、最终host/bridge/离线及许可闭包，经审核录入生产矩阵；P1自身产品功能另验 | 首测设备不代表整个SoC家族，CPU不代表GPU/NPU |

B3b新增fixture审查记录至少含review_id、证据报告/资产hash、完整BuildIdentity、模型/策略/suite身份、具体设备档、允许参数与结论边界。证据更改后重新审核，fixture不自动追踪“最新”。B3a尚有失败/缺测时只能按明确审定的研究范围推进，不得将未通过项标为通过；生产准入始终要求对应必要门槛。

本片完成记录区分：文档契约、集成构建、APK静态审计、设备B3a、B3b、生产矩阵、P1功能。新工具依赖许可、磁盘空间不足或设备不可用分别上报；不复用早先一次性环境/网络授权作为永久构建权限。
