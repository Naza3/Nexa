# 共享core：取消与真实故障归因修正

日期：2026-10-02。实现范围仅runtime-core调度器及回归测试；不修改公共DTO、原生ABI或Windows进程隔离边界。

## 原问题与修复

安卓B3b接入检查发现Job::terminate优先读取取消reason，导致取消或shutdown先到、真实NativeFailure后到时错误地发布Cancelled；PC engine-host同样可能通过GenerationFailed上报真实错误。旧实现在新增cancel→GenerationFailed(NativeFailure)确定性回归上失败（exit101），修复后通过。

普通取消四类reason遇非控制真实Err时保留Failed原错误。控制确认包括四类取消及Queue/Load/ExecutionTimeout；PC安全回收后的强杀取消仍Cancelled。已有deadline/协议失败reason保持原优先级，CleanupUnconfirmed仍最高，断开消费者不强行发送终态。

加载请求可能已提前Cancelled，之后收到真实load故障时不能重写该请求终态。abandoned load仅将控制原因视为正常放弃；本次shutdown期间晚到真实load/generation/unload错误单独保留，始终先调用close，返回优先级：cleanup未确认 > close错误 > 本次首个非控制错误 > 成功。历史已恢复故障不污染以后shutdown。通用调用方不能仅凭shutdown错误码推断close成功；具体执行器的清理证明单独核验。

## 本地验证

- 新确定性反例在旧实现exit101；最终core 14单测＋40调度测试通过
- 合成时序覆盖cancel与fault双顺序、GenerationFailed/Faulted、shutdown安全ACK、新故障传播、load已取消后晚故障、无request卸载错误、正常PC控制ACK、cleanup最高、历史恢复、断流和唯一终态
- 七crate聚合：runtime-core、runtime-types、model-store、runtime-ipc、process-host、runtime-api、desktop-bridge共226通过、0失败、0忽略，exit0
- 分项：core54、desktop55、store35、process24、api42、ipc8、types8；Linux真实子进程故障/containment与HTTP/bridge控制测试已执行
- 同七crate clippy --all-targets -- -D warnings、core cargo fmt及git diff --check均exit0；root独立重跑core54项通过；远端CI另行记录
- 独立设计复核无阻断；未运行本机Windows专属cfg用例，未配置GGUF/MNN真实模型，本地结果不替代Windows、MNN native CI或手机B3b

使用独立target目录避免干扰并行App构建。首次offline因锁内ambient-authority尚未缓存退出101；随后官方registry按Cargo.lock补依赖后聚合成功，没有改变锁文件。

## 后续门槛

本片改变共享core行为，必须触发并核验完整Windows与Android MNN native回归，不能按Android-only跳过。B3b验证器另行实现/审查/构建；此记录不宣称其已通过设备验收。


## 首次远端回归与路径测试修正

实现提交99aeba44af146a3e8f805d58abbfb0709ddd37ff已推送。Android MNN native运行37004301790在tools/index8失败，尚未进入模型或编译阶段；下载失败证据Artifact11225486323（1,382 bytes，SHA256 cc0c0e098386624d6c6b97ece602b16c5af34af3b3c186532cbf70855a19ff74）后本地精确复现：Windows路径过滤helper测试期望未同步07a已批准的apps/android-verifier/**隔离项。

修正只更新该测试期望，保留严格列表核对并补真实runtime-core/types、相邻目录和App+core混合变更反例，没有扩大工作流忽略范围或放宽native门禁。失败原运行保留，新提交触发Android重验；Windows99a运行独立继续。


Android现有回归收尾：a67c109的prototype37004645545于12:20:19 UTC成功，artifact11225662836（9,316 bytes，SHA256 90f036d4e3e9a1135962d5440967421e6e0cdff336ba9a667997fc90987b245b），下载后13份报告与最终门禁独立复核。native37004645523于12:33:37 UTC成功，artifact11226058096（13,299 bytes，SHA256 bf239076f7feff4eabe4750ab2502e46391a5a688fdded7a3893c0be814125ea），12阶段及四项真实B2通过；root核对所有schema/source/原文proof、同次receipt和实际Git提交源码快照摘要e13b3affb5b50df2b7f0bf00240337890df8f5872900d93cc3cdc334cac557c9。receipt摘要9302135e31213fcdb573f0418d5f71d486dc2b6360193a08b8c415d8d52f56e9。两者android_run=false；这是已启动共享core回归的收尾，不代表恢复已暂停的Android App开发。


Windows共享修复99a回归37004301787于12:47:16 UTC成功（attempt1）。下载证据artifact11226997433，77,639 bytes，SHA256 47e00b298b2d865c580342f68801aeb7b8852cbdcf5b6fc2ec163b17a530a6cb；root独立核对50项库存大小/hash、精确source99a/tree06c70与全部成功结果。真实模型/取消恢复/HTTP/CLI、仓库外提取产品与desktop bridge通过，desktop package_unchanged=true、native_window_tested=false；失败路径bridge-failure.json及旧optional rustfmt日志缺失边界与之前一致。本片共享回归已闭环，不扩展原生UI/Windows11/离线/长期条件。
