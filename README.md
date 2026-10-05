# Nexa

Nexa 是面向 Windows 桌面处理器的本地大模型运行时。以固定版本 llama.cpp/GGUF 为推理核心，通过本机 API 为其他应用提供能力；桌面界面负责模型与服务管理，聊天用于调试和验证。

当前主目标是 Windows 10 x64 / Intel Core i5-8400 / 16GB 内存；后续按真实设备扩大 Intel、AMD 桌面 CPU 支持并验证 Windows 11。目标硬件不等于已经验收。当前树只维护桌面产品；项目移动实现与专用文档已按[ADR0029](docs/decisions/0029-desktop-only-source-tree.md)移除，Git 历史、独立项目与完整上游源码不变。

## 已有能力与边界

- llama.cpp 原生推理、Rust 单模型调度、有界队列、生成及加载取消、超时/空闲策略、独立 worker 与崩溃隔离
- 本机 HTTP 文本接口、SSE、非流式响应、CLI 与鉴权；可选局域网文本 API 默认关闭、独立凭据，不开放本机管理接口
- 桌面模型库、选中文件零复制登记、混合目录诊断、受控 aria2 下载、配置/模型档案、服务控制与聊天验证
- 合规单文件 GGUF 可尝试加载，不设特定型号/hash 许可名单；结构、完整性、原始模板、执行方式与资源检查仍必需，精确[模型矩阵](docs/model-matrix.md)只记录实测证据
- 关闭窗口与停止服务分别处理；托盘、开机自启动、生产工具调用、完整 Harness 工具闭环、GPU/NPU 及其他系统仍不属于已实现承诺

## 最新已交付版本

2026-10-05 已交付版本 0.1.0 的便携 ZIP、MSI 与 Setup EXE，精确来源为 `de7732f031c11e44a27f86b33a341c48131a3906`。[Windows Actions37306309927](https://github.com/Naza3/Nexa/actions/runs/37306309927)通过真实模型/桌面 bridge 与全部 13 项安装生命周期门槛，随后独立产物审计通过。三格式来自同一 28 文件 payload，完整 10 份许可及 aria2 对应源码保留。

这是开发分支构建的交付，实际 GitHub tag 发布尚未执行。安装器未签名；Server 2022 CI 与 Setup 向导通过不代替用户 Windows 10、应用原生窗口、两机 LAN、干净机器/离线或长期稳定性验收。精确文件/hash和证据边界见[三格式验证记录](docs/verification/2026-10-05-tag-release.md#最终de7732f原生成功与三格式交付)。随后开展的桌面源码清理另记[当前状态](PROJECT_STATE.md)，不把旧包当作清理后的新包。

## 交付与使用

| 产物 | 内容 |
| --- | --- |
| `Nexa-0.1.0-windows-x64-portable.zip` | 完整解压运行的桌面、runtime、下载组件与许可 |
| `Nexa-0.1.0-windows-x64-setup.msi` | 当前用户 Windows Installer 安装/修复/卸载 |
| `Nexa-0.1.0-windows-x64-setup.exe` | 内含同一 MSI 的离线安装向导 |
| `windows-x64-cpu.zip` / `acceptance-tools.zip` | CI 另有独立 runtime 包与验收工具，不是额外安装前置 |

模型外置；安装、修复、升级及卸载保留用户模型和配置。桌面需要已安装 Microsoft WebView2 Evergreen，缺失时提供官方入口，不自动安装。使用与安全边界见[安装器说明](docs/windows-installers.md)、[Tag 发行流程](docs/windows-releases.md)、[桌面包](packaging/desktop-windows/README.md)、[runtime 包](packaging/windows-x64-cpu/README.md)和[局域网使用说明](docs/lan-api-usage.md)。

## 当前方向与阅读入口

后续桌面性能、目标机和发行验证按[Windows 路线](docs/roadmap.md)推进。官方 DeepSeek Harness（dsh）的 pi-ai provider 是 API 兼容目标，当前严格文本子集不等于完整工具协议；具体差异见[Harness 契约](docs/windows-harness-contract.md)。Telegram 摘要只是可选参考调用端，不构成 runtime 发布前置。

1. [开发规则](AGENTS.md)、[当前状态](PROJECT_STATE.md)、[项目索引](PROJECT_INDEX.md)
2. [架构](docs/architecture.md)、[Windows 范围 ADR0014](docs/decisions/0014-windows-desktop-cpu-runtime.md)、[桌面源码边界 ADR0029](docs/decisions/0029-desktop-only-source-tree.md)
3. [runtime 执行规格](ai-runtime-v0.1-execution-spec.md)、[构建锁](docs/build-lock.md)、[模型矩阵](docs/model-matrix.md)、[验证命令](xtask/README.md)

产品名 Nexa；已有 `ai-runtime`、`ai-runtime-worker` 和 `air_*` 名称保持。
