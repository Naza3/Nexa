# Nexa

Nexa 是面向 Windows 桌面处理器的本地大模型运行时。以固定版本 llama.cpp/GGUF 为推理核心，通过本机 API 为其他应用提供能力；桌面界面负责模型与服务管理，聊天用于调试和验证。

当前主目标是 Windows 10 x64 / Intel Core i5-8400 / 16GB内存；后续按真实设备扩大 Intel、AMD 桌面 CPU 支持并验证 Windows 11。目标硬件不等于已经验收。Android 设计已移出主线，历史代码、证据和未提交研究工作保留；GPU/NPU、其他系统和 Telegram 业务不是当前发布前置。

## 已有能力与边界

- llama.cpp 原生推理、Rust 单模型调度、有界队列、取消/超时、空闲卸载、独立 worker 与崩溃隔离已有实现
- 本机 HTTP 文本接口、SSE、非流式响应、CLI 与鉴权已实现；不是全量 OpenAI 或 DeepSeek 协议兼容承诺
- 桌面已有模型目录选择、零复制登记、GGUF 自动名称、兼容性原因、聊天/停止、参数设置和服务启停
- 默认正常关闭窗口保留 runtime，另有同时退出选项；托盘管理器仍属后续计划
- 开放模型加载增量已完成本地实现/回归，待新提交Windows验收：未实测的合规单文件GGUF可尝试，由llama实际加载判断；不设特定型号/hash许可名单。精确[验证矩阵](docs/model-matrix.md)只记录证据，不再作为产品模型清单。已交付389eeef仍是旧限制，新行为尚未交付

最新已交付 Windows 包对应实现提交 `389eeef`，已通过 [Windows CI37009292638](https://github.com/Naza3/Nexa/actions/runs/37009292638) 的真实模型、完整包与桌面 bridge 回归。新包的 Windows 10 原生目录操作、自动名称、零复制与剪贴板等分支仍待手工验收。旧包核心 UI 的手验不追溯证明新功能。详见 [当前状态](PROJECT_STATE.md) 与 [交付记录](docs/verification/2026-10-02-windows-model-compatibility.md)。

## 当前方向

后续按 [Windows 路线 W00–W05](docs/roadmap.md) 推进：W00范围收敛后，W01现版短验与W04 Harness文本互通并行；W02先开放模型候选，再用基准样本测16GB机器的资源/质量，真实工具闭环仍需独立模型工具能力证据。W04文本优先于非必要托盘美化，不被W03管理器阻塞；各阶段验收齐备后再进入W05后期发行验收。

API兼容目标已确认为官方DeepSeek Harness（dsh）：优先用其pi-ai自定义openai-completions provider连接Nexa。准确版本、工具协议差异和H01–H12门槛见[harness契约](docs/windows-harness-contract.md)。当前文本子集仍不足以完成agent工具闭环，不宣称已经兼容。

无开发工具、实际离线、长期稳定性留在后期验收；当前开发不由这些未完成条件阻塞。Telegram 摘要只是可选参考调用端。

开放不等于全部GGUF保证可用：结构、原始模板、执行方式与设备资源仍有限制。当前目录登记仍整批原子完成，一个不支持文件可使整次扫描失败；逐文件诊断属于下一片。当前新增的managed/external单文件16GiB安全预算不是16GB内存能装下模型的保证；metadata窗口小于默认2048的登记限制也尚未改进。详细设计见[ADR0015](docs/decisions/0015-open-model-loading-and-validation-evidence.md)，实现/验收进展见[W02记录](docs/verification/2026-10-03-windows-open-models.md)。

## 交付与使用

| 产物 | 内容 |
| --- | --- |
| `windows-x64-cpu.zip` | API/CLI、独立 CPU worker、必要运行依赖和许可 |
| `desktop-windows.zip` | Tauri 桌面界面与同源码 runtime 子目录 |
| `acceptance-tools.zip` | 独立短验工具，不是 runtime 运行依赖 |

模型外置。桌面包需要已安装 WebView2 Evergreen，缺失时提供官方入口，不自动安装。使用说明见 [桌面包](packaging/desktop-windows/README.md)、[runtime 包](packaging/windows-x64-cpu/README.md) 和 [独立验收工具](xtask/PACKAGE_ACCEPTANCE.md)。

## 阅读入口

1. [AGENTS.md](AGENTS.md)、[当前状态](PROJECT_STATE.md)、[项目索引](PROJECT_INDEX.md)
2. [架构](docs/architecture.md)、[Windows 范围 ADR0014](docs/decisions/0014-windows-desktop-cpu-runtime.md)
3. [runtime 执行规格](ai-runtime-v0.1-execution-spec.md)、[Windows 路线](docs/roadmap.md)
4. [构建锁](docs/build-lock.md)、[模型矩阵](docs/model-matrix.md)、[实际验证命令](xtask/README.md)

产品名 Nexa；已有 `ai-runtime`、`ai-runtime-worker` 和 `air_*` 名称保持。历史混合平台设计见 [归档索引](docs/archive/windows-focus-2026-10-03/INDEX.md)，不再作为当前实施指令。
