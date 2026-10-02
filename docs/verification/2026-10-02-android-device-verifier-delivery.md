# Android B3a 设备验证版：构建、CI 与首轮交付

日期：2026-10-02 UTC。状态：研究 APK 已构建、独立复核并发送；用户已回传固定设备的两份自动测试与一次后台取消报告。自动CPU矩阵通过不等于完整设备验收、生产模型准入或GPU/NPU支持。

## 精确源码与安装包

- 开发分支 `codex/nexa-native-baseline`
- 源码 commit `c0c0927b6a576f296577bc4db6e35039fe118db1`，tree `c12fc577c4bab410f7b153e64a732d0602e69cfd`
- 提交后从干净源码重建，实际 Android Cargo rustc-env 为该 commit、source_dirty=false、release；成品桥接 ELF 包含对应源码身份
- `Nexa-device-verifier-0.1.0-c0c0927-arm64.apk`：27,882,858 bytes；SHA256 `6cc75240c4b13a391005e8c1f83d74929bdd6bef9c5c09215785a8908064c026`
- 直接 APK 附件因大小限制未成功发送；改用只包含同一 APK 的 ZIP，解压后的字节与上述 APK 精确一致
- `Nexa-device-verifier-0.1.0-c0c0927-arm64.zip`：12,057,217 bytes；SHA256 `933473a14ff73709e00de2ca4b778f4c50e45fa1c8b021cd0c40806135adeafb`
- 2026-10-02 09:05 UTC 附件发送被接受，不等于用户已下载、安装或完成测试

应用名“Nexa 设备验证”，applicationId `io.github.naza3.nexa.verifier`，version `0.1.0+1`。release 编译、内部 Android Debug 签名；没有正式发行签名承诺。验收步骤见[一加15首轮指引](../android-device-verifier-acceptance.md)。

## 最终 APK 独立复核

最终审计 JSON SHA256 `f489877852d9d678ee6ed27f2dafcc23ffd89daf4b641f153a59aa32099bc122`。

- V2签名、ZIP16KiB对齐、仅arm64三库、37项预构建源码输入hash通过
- minSdk28/targetSdk36；研究用途true、生产准入false；无INTERNET或广泛存储权限，备份关闭
- 签名证书SHA256 `36025a8400c7dfdd9b87c8ee9b4420e39f199f9ce3ad1edf4a4100905b90c1ce`
- 登记桥接so SHA256 `d1855d284f540cd629ac2b9113ad142d024e858f0d6e835c023604888f35551d`
- APK内strip桥接so SHA256 `9dfa62ca060000102b9a3ad610fda8d2ce096904eb6c6933e2d3d7aee9eb3b2c`；与对登记so按NDKr30 strip所得字节一致
- 桥接库仅依赖libc/libdl/libm，无额外MNN或动态C++库；JNI和FRB均位于同一桥接so
- 自建桥接LOAD16KiB、RELRO末端严格16KiB。Flutter engine RELRO按Android linker设备页外取整不覆盖额外RW数据
- 固定Dart AOT libapp.so LOAD64KiB，确实无GNU_RELRO；无relocations/NEEDED/GOT/PLT，仅96B.dynamic和8B.bss可写。保留此vendor例外，不能宣称全部.so都有RELRO
- 主notice与已审原文及APK资产逐字节一致：4,882,579 bytes，SHA256 `9fc02079a5a04ab144c290c41465c294998efe682024ff7f23d2ec1d4b0306be`；含native、Rust、Flutter/Dart/FRB与43个去重Maven runtime archive对应内容

ELF完整区间、源许可闭包、控制层异常修复及本地真实链见[App验证记录](../../apps/android-verifier/VERIFICATION.md)。该记录中的预提交APK摘要属于历史包，不替代本节最终提交绑定摘要。

## 精确提交的 Android CI

两套运行的head均为c0c0927，tree均为c12fc577；下载后独立核对ZIP/hash、schema、源身份及必需报告，不仅查看绿色状态。

- [Native CI36984549770](https://github.com/Naza3/Nexa/actions/runs/36984549770)：09:00:13 UTC success；12阶段、同次B1→receipt→四项B2真实测试、Android五ELF链接和最终门禁通过
  - Artifact11218435023：ZIP13,278 bytes，SHA256 `09f7e80797ad4bdaa795297478f075ffa7e768f21d2dc5bb539338ef9e6c84d4`
  - source_snapshot SHA256 `dc49c70a928e12ad2148eebb8c8b3a2f70aa54c41d36a7e88ec9cd577a97f278`，与本地同提交干净源码重算一致
  - receipt SHA256 `62766974f5f8c8badab7a4d999c0d9d3385efa58e285b6585f444a6ba0a5d4cf`；七份上传proof原文hash与B2绑定独立复验
  - Ubuntu Linux artifact manifest `9698698f942ff64c93e15927e9ea212b69dae90844e7dc3efbb6e00ee22005c5`；Android CI artifact manifest `6d225d696c039e29a0fb0617f625fb98c66bc7d48f1073c07acabafd846e0391`
- [Prototype CI36984549775](https://github.com/Naza3/Nexa/actions/runs/36984549775)：08:45:31 UTC success；13份报告独立验证通过
  - Artifact11217407310：ZIP9,317 bytes，SHA256 `d6752b9f5596495e28e502380c51f098838475cdc08e8e40eebfe587409b2f08`

两者source_clean/all_required_steps_succeeded/evidence_verified均true，android_run=false。CI与本地构建指纹分别记录；不把跨环境archive摘要不同掩饰为逐字重现，也不把native CI称为手机执行。

## Windows 与尚待验收范围

同提交[Windows回归36984549760](https://github.com/Naza3/Nexa/actions/runs/36984549760)于09:21:32 UTC success，job110766477070全部必需阶段通过。Artifact11219225573的ZIP77,378 bytes，SHA256 `8847e729ace0e317aadd4b837b7260c21724e1e14c7c00d554a08b1edd97f20e`。独立核验精确c0c0927 source、50个indexed文件大小/hash、封闭库存与成功状态；HTTP50次断连恢复、runtime/native、完整Release产品及解压后的desktop bridge验收通过。成功路径未生成failure-only bridge-failure.json，旧可选windows-rustfmt.log也未列入当前库存，不能声称所有历史文件均存在。

桌面acceptance=result pass、package_unchanged=true、native_window_tested=false；未下载或重新交付Windows产品包。此结果不扩大原生窗口、Windows11、无开发工具、离线或长期稳定性范围。

首轮交付时上述设备项目尚未执行；后续用户报告的已验证范围与发现见下节。SAF provider故障、Activity完整重建/进程重启、无自动重放独立确认、内存温度/持续性能与日志canary仍未完整验证。固定模型五文件须另行准备；不能读取MNN Chat私有/data缓存。长期稳定性、完整聊天产品、任意模型和GPU/NPU后端不属于本次已完成范围。

## 四文件模型附件

用户后续改为不打包权重文件，并明确这次下载不通过GitHub。2026-10-02 09:19 UTC直接发送四文件ZIP被接受：`Qwen3-0.6B-MNN-Nexa-four-files.zip`，1,445,429 bytes，SHA256 `3ccc5ed5d485a4de1f1310d70913dbbc3ee808317eb8d7c74f8dfafd4a30584c`。ZIP根目录仅有config.json、llm_config.json、llm.mnn、tokenizer.txt；源文件与ZIP读回的大小/hash均与固定revision锁一致，CRC通过。没有.weight、额外目录或说明文件；须另行配合同版本llm.mnn.weight，不能将这四文件单独作为完整模型运行。


## 后续用户报告与修正项

同一c0c0927构建的smoke与safety两份重跑报告均为17项自动用例passed、6项人工或后续项目not_run，cleanup=confirmed。smoke总结果passed；safety因尚未完成手工项等按设计为inconclusive，不得写成完整安全矩阵通过。取消恢复、中英文/多轮、精确预算、stop与各公开checkpoint取消已在固定设备报告中通过。构建身份、suite源码摘要与五模型文件锁均已独立核对；报告为用户回传、自报环境证据，不是远程认证。

用户随后按约定执行切后台并明确观察到取消；对应报告outcome=cancelled、unload_close=passed、cleanup=confirmed。这支持本次后台取消及安全卸载，但没有独立确认所有重建/重启/无重放路径。保留所有原始报告，不修改not_run为passed，也不将中途中断的早期报告计为完整通过。

报告暴露两个待修正App问题：Dart对仅失焦的inactive也发取消，超出了“不可见时取消”的既定边界；普通生成被取消时runner泛化成native_failure，封存又覆盖更具体取消原因。正在仅App内修复，并要求真实故障与cleanup_unconfirmed不能被并发取消掩盖。新行为需要定向回归与新构建验证，不能直接继承c0的全部结论；生产resolver准入保持不变。

六项not_run分别保留：手工后台动作、SAF故障、进程重启、外部日志采集、未实现长期稳定性与尚未准入的B3b/core。后台动作另有用户观察与取消报告，不能伪造旧JSON中的自动case标记。


## 0.1.1+2 修正片的预提交验证

18项Rust控制测试、7项Dart/实际observer回归、clippy/analyze和56.38秒真实host闭环通过。独立审查另复验6项runner与协议故障顺序测试共7项，均通过。核心规则：普通用例只把具有操作取消上下文的原生Cancelled当取消；预期内部cancel必须实际发起且到达对应检查点；真实故障覆盖先到取消，后到取消不能覆盖故障，cleanup_unconfirmed始终最高优先级。

默认导出`.txt`，内部仍为不可变JSON原字节；未增加上传、分享、日志或权限。pubspec版本0.1.1+2是报告版本单一来源，并加入预构建输入hash，签名与applicationId保持以便覆盖安装。源许可/FRB生成件/锁文件及native构建资产未改变。最终提交包与最短手机补验另行绑定，不把预提交包当作交付版本。

Windows工作流仅新增精确`apps/android-verifier/**`路径排除，与独立Android工作区边界一致；共享core/types、根Cargo、桌面源码与Windows工作流本身仍触发回归。已检查正负路径样例；本次修改工作流自身仍会触发Windows验证。


## 07a14d2 最终修复包与设备补验

修复实现提交 `07a14d25c3d2c3b109014d7b35c5810f0e1694c2`，tree `13a0069b594875cbece3afc3bc0ecbec8788ced5`。最终干净构建为0.1.1+2，38项源码输入、包内版本、签名与源码身份独立复核通过；签名/applicationId不变。2026-10-02 10:15 UTC直接发送ZIP被接受，后续三份报告均绑定此版本/提交，不能把旧c0报告改记为新包结果。

- APK：27,905,682 bytes，SHA256 `74ec444edc307a6d27ee22bdb08718164428e0a8f808c9bce0ea7af907d07bc0`
- 单APK ZIP：12,065,487 bytes，SHA256 `0d6dedb4de7f8fda336089ee47facd1e4c319fe33a2aa3f5ca8b160338ab1d35`；CRC及解包字节一致
- 审计摘要：`a260ff4264e13f56e58f0a5772319e8f7e33dc637052cb097593af86a5887c4d`
- 包内桥接so：`9b759e156629cfd1fa7dd1d7d8f9024b92d292021e92a2eebf6f00380655d59b`
- smoke suite摘要：`9eb7c6123608c9b7a913649e41d0eb082a526dc64144ef3f1d26653aad6eace2`，由该提交runner源码独立重算

用户按短暂失焦、切后台、手动恢复的指引回传三份TXT报告。原始报告保留在私有验收材料，不纳入仓库；以下摘要仅供追踪一致性，不构成设备认证。

- focus：17 passed、6 not_run，总体passed、error=null、cleanup=confirmed；支持本次短暂失焦流程。报告文本SHA256 `e8ba89684346932de4a80967b46d791c8f36808ceb72e3a544cc11f3014a785c`
- background：8 passed、15 not_run，总体cancelled、原因backgrounded、cleanup=confirmed；已不再误记native_failure。此样本在adapter阶段前停止，不能声称精确中断某个原生kernel。摘要 `8d5a166be3ed0d02c43d0a08bcd88797a5e968103d42780c4a46c505c416e234`
- recovery：13 passed、1 inconclusive、9 not_run，总体cancelled/backgrounded、cleanup=confirmed。重新加载、中英文生成、活动取消恢复等已成功，足以确认本次恢复能力；后续adapter_cancel_template受后台动作打断，不能将整套结果改为passed，也无需为此要求用户重复整套验证。摘要 `1d75a3586831685e4a001765905958d8c66e852b307ae666bad59415a4b1d68a`

2026-10-02 11:36 UTC用户明确确认回前台没有自动重跑，只有手动点击才启动recovery；此为本次人工观察，不扩展到全部重建/进程死亡路径。B3b私有研究域二次审核结论见下节；生产resolver继续拒绝候选。4KiB单设备CPU证据不扩展为16KiB设备、全部芯片、GPU/NPU或完整安全矩阵。

## 07a14d2 Windows 回归闭环

[Windows CI36993778258](https://github.com/Naza3/Nexa/actions/runs/36993778258)于2026-10-02 10:44:40 UTC成功，attempt1/job110795722582。源码精确绑定上述07a提交；Artifact11222576521下载ZIP为77,358 bytes，SHA256 `f3e165170ebe115df46dcd2fde8959191fa2e50e9fdf56ffbe331ae9c28d0f53`。

下载后独立核对50个indexed文件的大小/hash、封闭库存、source与全部ci_step_outcomes；原生/真实模型、Release产品、解压包及desktop bridge验收成功。desktop acceptance=result pass、package_unchanged=true、native_window_tested=false。两项旧可选/失败路径报告缺失与c0一致，不影响此次通过；未重新交付Windows产品，也不扩大Windows11、无开发工具、离线或长期稳定性结论。


## B3b 私有研究域二次审核

2026-10-02 11:39 UTC完成独立审核，review_id `android-b3b-cpu-07a14d2-20261002`。上述最终APK、38项源码输入、三份报告与suite身份逐项匹配，11:36用户确认补齐无自动重放观察。按照ADR0011，允许实现验证器内部的受控core研究fixture/resolver；这是B3b运行资格，不是B3b测试通过或生产准入。

fixture固定上节07a提交/tree、APK/桥接及三报告摘要和b3a_smoke_v1摘要作为历史证据关联。新B3b包必须另留新构建绑定，不能要求其APK/桥接hash与旧证据包相等。当前native六字段、完整模型五文件/模板/来源/策略逐字取自受审focus报告并与原锁复核；运行时任何身份不匹配都拒绝，不接受导入报告或Dart白名单自授。

允许设备档仅为实际自报manufacturer=OnePlus、model=PLK110、soc_manufacturer=QTI、soc_model=SM8850、android_release=16、sdk_int=36、security_patch=2026-01-01、supported_abis=[arm64-v8a]、page_size=4096。固定模型qwen3-0.6b-mnn，artifact_digest `1ec59d439451738b4992f2ea5b06438788d752da81d55f11e1e8866d03fa7a57`；CPU/context2048/threads2/batch32/temperature0/top_p1/seed0/nonthinking，输入与max_tokens由版本化suite固定。

实现范围限定独立验证器crate：同snapshot私有resolver与原MnnExecutor接入原Runtime，生产resolver仍须拒绝。原EventLease必须一直保留至有效ack或明确丢弃，不复制信用账本；JNI/UI仅置停止并唤醒，同步core控制及shutdown留在工作线程。未捕获的真实队列窗口记inconclusive，断流后不伪造本就不再发送的终态。原生清理不确认永久fail-closed。

B3b现进入实现阶段，尚无B3b设备结果。完整后台各phase、Dart停止后的独立原生路径、SAF故障全集、重建/进程死亡、logcat、长期稳定性/温度/内存与16KiB设备仍未验证，保持对应后续门槛。


## 12:23 UTC方向调整后的暂停边界

用户要求回到Windows，安卓计划修改MNN Chat。独立验证器B3b源码WIP已停止，未提交、未发布，也没有新APK或B3b手机结果。作者已安全结束全部模型/测试/构建进程；32项Rust控制测试、clippy、Dart analyze与7项测试通过，B3a真实host55.22秒回归发生在最后ack小修正之前。最后ack原子退休补丁尚未独立复核，不把暂停源码标记可交付。见[ADR0013](../decisions/0013-windows-focus-and-android-mnn-chat.md)；既有0.1.1报告和限定研究审核作为历史事实保留。
