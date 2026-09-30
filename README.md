# Nexa

为自己的 Windows / Android 应用提供统一的本地大模型推理核心。首个业务验证场景是 Telegram 群消息摘要。

当前已开始 T00/T01 工程：最小 Rust workspace、自有 C++ ABI、真实模型原生探针及验证命令已落地。Windows/Android 产品、HTTP、调度和 UI 尚未完成；当前验证边界以 [PROJECT_STATE.md](PROJECT_STATE.md) 为准。

## 交付形态

| 层 | 计划交付 | 使用方式 |
| --- | --- | --- |
| 共享推理核心 | Rust 类型、调度、模型管理、llama.cpp 适配 | PC 与移动端复用源码和事件语义 |
| Windows runtime | 本机 HTTP 服务、CLI、独立推理 worker | 自有应用通过本机 API 调用 |
| Android 嵌入库 | 原生库、受控 Rust 桥、Flutter 接入示例 | 每个 App 内独立实例，前台运行 |
| 验证 UI | Tauri 桌面 / Flutter 移动最小界面 | 导入模型、生成、取消和检查状态 |
| 摘要参考接入 | 来源标准化、分块、证据引用与任务编排 | 调用 Nexa；不侵入推理核心 |

首版引擎采用固定 commit 的 llama.cpp；支持范围由模型、量化、模板、后端和设备实测组合决定。Windows i5-8400 / 16GB 与 Android 骁龙 8E5 / 12GB 是目标设备，不是已验收结果。

## 阅读入口

- 开发代理先读 [AGENTS.md](AGENTS.md)、[当前状态](PROJECT_STATE.md)，再按 [索引](PROJECT_INDEX.md) 定位。
- 了解总体方案：[架构](docs/architecture.md) 和 [范围决策](docs/decisions/0001-nexa-scope-and-layers.md)。
- 实现 runtime：[v0.1 执行规格](ai-runtime-v0.1-execution-spec.md)。
- 接入首个业务：[Telegram 摘要方案](docs/telegram-summary.md)。
- 持续实施与验收：[开发路线](docs/roadmap.md)、[代理工作流](docs/agent-workflow.md)。

## 开发起点

先按 [构建锁](docs/build-lock.md) 准备原生依赖，再使用 [当前验证命令](xtask/README.md) 运行单测与真实模型 smoke。精确模型来源及 hash 见 [模型矩阵](docs/model-matrix.md)。

`cargo test --locked --workspace` 和 `xtask baseline-verify/native-smoke` 已有实现；执行规格中的 `ai-runtime`、HTTP、移动与发行命令仍属于后续阶段，不能视为可用产品。

Telegram 消息来源和摘要触发方式尚未确定。消息获取、账号和摘要产物由应用层管理，模型推理可以离线完成；离线推理不表示 Telegram 数据获取无需联网。

产品名统一使用 Nexa。为保持已有规格一致，命令 `ai-runtime`、`ai-runtime-worker` 与 `air_*` ABI 暂沿用；实际包名及重命名通过决策记录管理。
