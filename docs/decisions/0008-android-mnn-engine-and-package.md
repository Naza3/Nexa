# ADR0008：Android MNN 引擎与模型包方向

日期：2026-10-02。状态：方向已采用；Android 源码、精确依赖、模型包、APK 和设备验收均未完成。本决策不认证任何 MNN 后端可用，也不改变现有 Windows 交付状态。

## 背景

既有架构以 Windows/Android 共用 llama.cpp adapter 和单文件 GGUF 为基线。Android 现改用 MNN，以 CPU、OpenCL GPU、QNN NPU 和直接 Hexagon 路线分阶段验证；无需先完成 Android llama.cpp。

当前 `EngineHost` 直接依赖 `llama-adapter`，`ResolvedModel` 与 manifest schema 1 仍面向单文件 GGUF。仅替换链接库不能满足新模型资产、模板、采样、取消和实际后端报告要求。源码入口、官方依据和逐项门槛见[Android MNN 计划](../t07-android-mnn-plan.md)。

## 决定

1. Windows 保留已锁定的 llama.cpp、GGUF、独立 worker、HTTP/CLI 和桌面行为。Android 采用 MNN 与独立原生适配；两端共享 Rust 类型/调度/事件语义及存储安全规则，不要求共用原生引擎或二进制模型文件。
2. Android 通过实现现有 `Executor` 契约接入同一 core，不复制调度器。MNN 对象由专用 owner thread 独占，只有独立原子取消控制跨线程；不强杀 native 线程，不提前确认资源释放。
3. MNN 采用多文件模型包：完整图/权重/tokenizer/模型配置/模板及变体引用闭包、路径约束、逐文件和整体身份校验、原子导入。不可变模型资产与可变运行配置/缓存隔离。旧 GGUF schema/证据含义保持；具体新 schema、C ABI 和公共 DTO 版本在 T07-B 实施前冻结，本决策不预先发布字段协议。
4. 模板缺失或不支持必须拒绝，不能拼接 messages 降级。每请求采样/seed 和精确 token 预算、真实 prefill/decode 取消、stop/UTF-8、背压与唯一终态必须独立实现并验证；上游 demo 不等于契约已满足。
5. CPU 是基础和默认路径；OpenCL、QNN v79/v81、直接 Hexagon 分开构建、profile、验证。QNN 与直接 Hexagon 使用不同转换/量化/产物及依赖闭包，不视作同一后端。真实混合执行/fallback 必须报告，生成开始后故障不自动重放。
6. 目标覆盖 Snapdragon 8 Elite 及后续代际；公开首测参考为 OnePlus 15 / SM8850 / v81，SM8750 / v79 为兼容档。实际 Android、ABI/页大小、内存和驱动由诊断确定；商品名或上游支持表不授予 Nexa 支持。
7. 工程顺序为精确版本/CPU 原型 → MnnExecutor/包/安全契约 → 最小前台 APK → OpenCL → QNN → 直接 Hexagon 实验。CPU Android 完成是基础门槛；GPU/NPU 纳入计划但不要求实验全部通过才交付 CPU。

## 保持的边界

单模型、单运行槽、有限 FIFO、独立请求 KV、精确模板后预算、无静默截断、后台取消并安全卸载、前台不重放，均沿用[执行规格](../../ai-runtime-v0.1-execution-spec.md)。Android 不引入 PC HTTP/IPC/process-host 或 llama 依赖；Windows API 不链接原生推理库。

不新增 Telegram/账号/云服务、聊天业务、后台常驻、模型市场或性能承诺。新增 SDK/工具协议、许可接受、安装和再分发范围另行核验并按授权处理；本文不授权下载 SDK 或改系统。

## 迁移与验收

- 本轮同步架构、规格、路线及入口的引擎/模型资产方向；已有源码仍保持 GGUF/llama，实现状态不随文档变化
- T07-A 锁定 MNN 完整 commit、工具链/导出器和真实小模型包；3.6.1 只是已查到的候选，未进入 Nexa 构建锁
- T07-B 先处理现有单文件/引擎耦合，再做公共类型兼容和 Windows 回归；不把新 MNN variant 发给旧 worker
- 继承 A01–A26 中移动适用项并补多文件/后端负例；native 探针、APK、真机短验、持续负载分层报告，未测项不得晋级
- Android 尚无工具链锁、模型包或实测结果；具体切片与停止条件见[计划](../t07-android-mnn-plan.md)，动态进度见[状态](../../PROJECT_STATE.md)

本决策替代旧规范中 Android 使用 llama.cpp、单文件 GGUF 及不纳入 MNN/QNN 的方向；不覆盖历史 Windows 决策或验证记录。
