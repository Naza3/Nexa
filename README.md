# Nexa

为自己的 Windows / Android 应用提供统一的本地大模型推理核心。首个业务验证场景是 Telegram 群消息摘要。

当前T00–T04已完成固定Windows CPU CI阶段验收：Rust核心、模型存储、原生ABI、独立worker、进程隔离与HTTP/CLI已落地。Windows 10为首要交付目标，Windows 11后续增加；T05私有CPU便携包与独立验收工具已通过固定Windows Server2022 CI及独立Windows 10手工短验。无开发工具、离线运行和长期稳定性留作后期验证，T05按当前范围已完成。T06最小Windows UI/安全bridge/独立桌面包已实现，Windows原生构建/完整桌面包及真实Release bridge已通过，旧包核心UI已有独立手工验收；新目录版CI及产物独立复核已通过并交付，新目录原生UI操作和其余UI分支仍待验证。Android仍未完成，实际边界以[PROJECT_STATE.md](PROJECT_STATE.md)为准。

## 交付形态

| 层 | 计划交付 | 使用方式 |
| --- | --- | --- |
| 共享推理核心 | Rust 类型、调度、模型管理；Windows llama / Android MNN 独立适配 | PC 与移动端复用控制层和事件语义 |
| Windows runtime | 本机 HTTP 服务、CLI、独立推理 worker | 自有应用通过本机 API 调用 |
| Android 嵌入库 | MNN 原生库、受控 Rust 桥、Flutter 接入示例（未实现） | 每个 App 内独立实例，前台运行 |
| 验证 UI | Tauri 桌面 / Flutter 移动最小界面 | 导入模型、生成、取消和检查状态 |
| 摘要参考接入 | 来源标准化、分块、证据引用与任务编排 | 调用 Nexa；不侵入推理核心 |

Windows 保留固定 commit 的 llama.cpp/GGUF；Android 主引擎采用 MNN/多文件模型包，先完成 CPU，再验证 OpenCL、QNN v79/v81 与直接 Hexagon。见[方向决策](docs/decisions/0008-android-mnn-engine-and-package.md)和[执行计划](docs/t07-android-mnn-plan.md)。支持范围按模型、量化、模板、后端和设备实测组合认定；Android 尚未实现或实测。固定 Windows CI 和短验结果不覆盖无开发工具、实际离线或长期稳定性条件。

## 阅读入口

- 开发代理先读 [AGENTS.md](AGENTS.md)、[当前状态](PROJECT_STATE.md)，再按 [索引](PROJECT_INDEX.md) 定位。
- 了解总体方案：[架构](docs/architecture.md) 和 [范围决策](docs/decisions/0001-nexa-scope-and-layers.md)。
- 实现 runtime：[v0.1 执行规格](ai-runtime-v0.1-execution-spec.md)。
- 接入首个业务：[Telegram 摘要方案](docs/telegram-summary.md)。
- 持续实施与验收：[开发路线](docs/roadmap.md)、[代理工作流](docs/agent-workflow.md)。

## 开发起点

先按 [构建锁](docs/build-lock.md) 准备原生依赖，再使用 [当前验证命令](xtask/README.md) 运行单测与真实模型 smoke。精确模型来源及 hash 见 [模型矩阵](docs/model-matrix.md)。

`cargo test --locked --workspace`、`xtask baseline-verify/native-smoke/api-smoke` 和 `ai-runtime` HTTP/管理CLI已有实现。T05新增Windows原生开发机上的 `cargo run --locked -p xtask -- build --platform windows-x64 --backend cpu` 与独立 `nexa-acceptance.exe`；构建/运行状态见[T05验证](docs/verification/2026-10-01-t05-windows-package.md)，独立Windows 10手工短验与CI结果分层记录，A20无开发工具/离线与长期稳定性留作后期验证，不阻塞当前UI开发。

产品包为 `windows-x64-cpu.zip`，独立验收工具为 `acceptance-tools.zip`，各自附SHA-256。解压后可用工具的 `--model <现有固定GGUF> --out <报告.json>` 做短验；产品路径变化时显式 `--package <目录>`。不要求用户安装开发工具，不包含模型、UI、用户数据或长期凭据。详见[便携包说明](packaging/windows-x64-cpu/README.md)和[独立验收工具](xtask/PACKAGE_ACCEPTANCE.md)。

Telegram 消息来源和摘要触发方式尚未确定。消息获取、账号和摘要产物由应用层管理，模型推理可以离线完成；离线推理不表示 Telegram 数据获取无需联网。

产品名统一使用 Nexa。为保持已有规格一致，命令 `ai-runtime`、`ai-runtime-worker` 与 `air_*` ABI 暂沿用；实际包名及重命名通过决策记录管理。


## T06 桌面开发入口

`apps/desktop/`已有React/TypeScript/Vite模型、聊天和设置三页，`src-tauri/`为独立Rust workspace。前端36测试/构建与Windows CI36864041027的真正Tauri构建、桌面包和真实Release bridge生命周期已通过。独立手工验收已确认旧包原生启动、导入、聊天、停止生成及两种关闭，剪贴板等其余分支仍待验证；同级外置GGUF误拒问题已定位；当前已接入设置选择只读模型目录、零复制注册和自动名称，本地通用链已验，源码75e458f的[Windows CI36948947690](https://github.com/Naza3/Nexa/actions/runs/36948947690)已成功，包含真实模型/runtime/HTTP/CLI、桌面包和解压bridge验收。下载产物独立复核已通过并交付；新目录选择、零复制和自动名称的原生UI仍未测，不能把bridge验收当作窗口操作通过。详情见[T06记录](docs/verification/2026-10-01-t06-desktop.md)。

桌面开发产物为另一个`desktop-windows.zip`，内含嵌入前端EXE与完整匹配的`runtime/`子目录，模型继续外置。需要已安装WebView2 Evergreen，缺失时原生提示官方入口，不自动安装或修改系统权限。默认正常关闭自身窗口保留runtime，不创建随UI关闭杀runtime的Job；外部宿主整体终止Job/会话后的存活不作保证。同时退出会影响所有客户端，须实际确认清理。使用与手工验收见[桌面说明](packaging/desktop-windows/README.md)，构建命令见[验证入口](xtask/README.md#t06-桌面构建与分层验收)。
