# ADR0034：统一推理执行超时与桌面聊天等待预算

日期：2026-10-08。状态：已采纳，源码已实现；实际检查与Windows目标机结果由本轮验证记录和[项目状态](../../PROJECT_STATE.md)分别说明。任务W02-OCR-4。

## 问题与决定

图片OCR的prepare尚未完成时，SSE响应头还未返回；图像编码和后续生成同样受既有runtime执行计时约束。只延长后端执行时间仍会被桌面文本聊天/图片OCR路径中的固定750秒等待截断。用户只需要一个“推理执行超时”设置，桌面根据当前服务的有效配置派生等待预算。

复用 `config.toml` 的 `runtime.execution_timeout_seconds`，不新增OCR专属设置。默认值仍来自 `SchedulingConfig` 的300秒；执行计时覆盖prepare、图像编码、prefill与decode，文件校验、排队和模型加载继续分别计时。

## 保存与兼容

统一 `RuntimePolicies` 新增必填 `execution_timeout_seconds: u64`。UI/API新保存范围1..86400秒；runtime整组JSON更新缺字段、null或类型错误必须拒绝，不用默认300秒重置已有值。范围错误返回 `configuration_invalid`，param为 `update.runtime.execution_timeout_seconds`；失败不发布任何配置修改。

runtime策略沿用停服、实例锁、配置锁和CAS保存，只在服务重启后生效。在线保存返回 `runtime_running`；磁盘外改继续分别显示saved与runtime_effective、标记pending_restart，不替换active预算。

旧TOML缺少该字段仍默认300秒；已合法读取的超大正数继续可读，保存其他组不新增整份配置范围限制。配置snapshot schema因此仅约束该字段minimum为1，runtime更新分支另约束maximum为86400。用户主动保存runtime组时须提供新的有效范围值。

## 只读预算与桌面连接

`runtime_effective.chat_response_timeout_seconds: u64` 为必填只读输出，由实际active配置依次计算：

```text
model_verification_timeout_seconds
  .saturating_add(queue_timeout_seconds)
  .saturating_add(load_timeout_seconds)
  .saturating_add(execution_timeout_seconds)
  .saturating_add(30)
```

默认总预算为1050秒。旧配置巨大正数相加时饱和至u64上限，GET不因此panic或失败；桌面使用单调时钟checked_add构造截止点失败时报 `response_invalid`。这不是可保存字段，不使用pending磁盘值。

桌面文本聊天与图片OCR先在同一已证明身份的Connection上GET `/runtime/configuration`，确认effective预算，再用该连接POST聊天请求。响应等待采用上述总预算，stream采用同一effective快照的 `execution_timeout_seconds + 30` 秒。移除这两个聊天路径的固定750秒兜底，缺失或无效预算明确失败，不静默改用旧值。设置、校验、加载等其他管理路径预算不变。

SSE事件和正文、输出/缓冲上限、request_id、取消、断流清理、单次终态与不自动重放保持原契约。时间预算变长不意味着扩大图片、上下文、输出或内存上限，也不保证任意图片成功。

## 操作与验证边界

用户在“设置”中将“推理执行超时”设为例如1800秒，停止服务后保存，再启动服务并显式重新加载模型，最后手动重新识别。旧包须整套升级为同源桌面与runtime产物，不能只替换UI或单个worker。

配置逻辑覆盖默认/必填、边界、精确错误参数、CAS、非法保存原子性、旧TOML兼容、在线/未授权拒绝、持久化与active/pending快照；预算覆盖非默认四阶段值与饱和。桌面协议测试应证明两个聊天入口使用effective值、缺失值失败以及取消语义保持。模拟协议、Linux检查、Windows原生构建和用户i5-8400实际OCR耗时分别记录，不把短时模拟当成真实1800秒OCR验收。
