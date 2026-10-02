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
