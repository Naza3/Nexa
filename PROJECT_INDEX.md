# Nexa 项目索引

本文件只负责定位，不将计划写成实现。动态状态见 [PROJECT_STATE.md](PROJECT_STATE.md)，当前范围见 [ADR0014](docs/decisions/0014-windows-desktop-cpu-runtime.md)。

## 当前文档

| 入口 | 用途 |
| --- | --- |
| [README](README.md) / [AGENTS](AGENTS.md) | 产品定位与开发规则 |
| [当前状态](PROJECT_STATE.md) / [Windows路线](docs/roadmap.md) | 已完成、待验与W00–W05依赖/门槛 |
| [架构](docs/architecture.md) / [执行规格](ai-runtime-v0.1-execution-spec.md) | Windows模块归属、API/IPC/原生与A编号验收 |
| [harness兼容契约](docs/windows-harness-contract.md) | 固定dsh/pi-ai接入路径、协议差异、H01–H12验收 |
| [开放模型ADR0015](docs/decisions/0015-open-model-loading-and-validation-evidence.md) | 用户16GB/广泛模型目标，loadable与历史validated分离，50c9d41固定模型WindowsCI及发送完成，用户目标机待验 |
| [混合目录ADR0016](docs/decisions/0016-mixed-model-directory-diagnostics.md) | 合法集合一次partial提交、完整有限诊断、全坏保旧及短context扫描；43ad5c2 WindowsCI/包复核及发送完成，用户目标机待验 |
| [发现/双源下载ADR0017](docs/decisions/0017-model-discovery-and-catalog-download.md) | 默认EXE/models发现、MS/HF固定目录、保存与扫描分离；33f0e17已发送，通用引擎另评估 |
| [aria2下载引擎ADR0018](docs/decisions/0018-generic-download-engine-candidate.md) | 已采纳受控aria2；工作树监督/发布/组件集成与下载socket/SChannel边界，最终Windows及新包待验 |
| [工具契约草案](docs/windows-tools-contract.md) | 生产tools尚未实现；T0已有无模型parser观察并定位严格完整性/schema/普通文本缺口 |
| [构建锁](docs/build-lock.md) / [模型矩阵](docs/model-matrix.md) | 固定工具链/llama与精确模型验证证据，非运行许可名单 |
| [代理协作](docs/agent-workflow.md) | 单写入者、检查和交接 |
| [历史索引](docs/archive/windows-focus-2026-10-03/INDEX.md) | 收敛前主文档、Android/MNN研究与历史状态，不驱动当前排期 |

## 实际 Windows 工程

| 路径 | 作用 / 说明 |
| --- | --- |
| `Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml` | Windows runtime主workspace与锁 |
| `crates/runtime-types/`、[runtime-core](crates/runtime-core/README.md) | 自有DTO、单actor、队列/取消/deadline/资源治理 |
| [model-store](crates/model-store/README.md) | managed复制导入与external只读目录、manifest/准入 |
| [llama-adapter](crates/llama-adapter/README.md)、`native/llama-shim/`、`vendor/llama.cpp/` | 固定llama原生集成与安全边界 |
| [engine-host](crates/engine-host/README.md) | 专用推理线程，供worker使用 |
| [process-host](crates/process-host/README.md)、[runtime-ipc](crates/runtime-ipc/README.md)、[runtime-worker](crates/runtime-worker/README.md) | 独立进程、Job/回收、NDJSON/信用与原生执行 |
| [runtime-api](crates/runtime-api/README.md)、[runtime-cli](crates/runtime-cli/README.md) | 本机HTTP/SSE与命令，父端不链接原生库 |
| [desktop-bridge](crates/desktop-bridge/README.md)、`apps/desktop/` | native-free桥与React/Tauri模型/聊天/设置页 |
| `apps/desktop/src-tauri/` | 独立Rust workspace/锁、ACL、原生Windows壳 |
| [xtask](xtask/README.md)、`tests/fixtures/`、`scripts/` | 实际可执行验证、真实输入与打包脚本 |
| `crates/desktop-bridge/src/model-catalog.json`、`download.rs`、`crates/model-store/src/library_download.rs` | 进行中的固定双源元信息、显式有界传输及受保护文件事务；不授予模型能力 |
| `crates/download-engine/`、`third_party/aria2/`、`scripts/build_aria2_windows.*` | 工作树受控sidecar监督、三补丁来源锁/构建与组件身份，HTTP/Range由aria2负责；最终Windows待验 |
| `native/llama-shim/tests/tool_parser_test.cpp` | T0合成上游模板/parser诊断，13case/无权重，不是生产工具接受算法 |
| `.github/workflows/native-windows.yml` | 授权开发分支Windows CPU真实构建/模型/包回归 |

`RuntimeConfig::android()` 等遗留源码、独立移动workspace和隔离CI仍原位保留，不因本次文档收敛修改。`.codex/agents/`是开发代理配置，不是产品运行依赖。

## Windows 接口、发行与原生窗口

- [HTTP/管理 ADR0005](docs/decisions/0005-t04-loopback-http-and-management.md)：鉴权、同连接proof、原子导入/输出预算与关停
- [worker ADR0004](docs/decisions/0004-t03-process-isolation-and-credit-ledger.md)：进程隔离、单一信用账本与消费lease
- [模型/调度 ADR0003](docs/decisions/0003-t02-scheduler-storage-and-observability.md)：存储、单actor与可观测性
- [便携包 ADR0006](docs/decisions/0006-t05-windows-portable-package.md)、[runtime说明](packaging/windows-x64-cpu/README.md)、[独立验收器](xtask/PACKAGE_ACCEPTANCE.md)
- [桌面契约](docs/t06-desktop-contract.md)、[壳ADR0007](docs/decisions/0007-t06-desktop-shell-boundary.md)、[目录/自动名契约](docs/t06-model-directory-contract.md)、[桌面包说明](packaging/desktop-windows/README.md)
- `scripts/package_windows.py`、`scripts/package_desktop_windows.py`、`scripts/run_desktop_smoke.py`：真实Release/PE/许可/hash、中文空格解压与bridge
- `crates/llama-adapter/native_identity.rs`、`scripts/stage_ci_evidence.py`：精确原生构建身份与封闭脱敏证据

## 已有验证入口

| 阶段 | 报告 |
| --- | --- |
| T00/T01 | [固定原生基线](docs/verification/2026-09-30-native-baseline.md) |
| T02 | [存储/调度/真实执行](docs/verification/2026-10-01-t02-runtime.md) |
| T03 | [独立worker与进程回收](docs/verification/2026-10-01-t03-worker.md) |
| T04 | [HTTP/CLI](docs/verification/2026-10-01-t04-http-cli.md) |
| T05 | [Windows便携发行阶段](docs/verification/2026-10-01-t05-windows-package.md) |
| T06 | [桌面及目录版CI/手验界限](docs/verification/2026-10-01-t06-desktop.md) |
| 后续修复 | [取消/真实故障优先级](docs/verification/2026-10-02-core-cancellation-faults.md) |
| 历史389eeef | [模型兼容性与最终CI/交付](docs/verification/2026-10-02-windows-model-compatibility.md) |
| W00 / W04 | [Windows主线收敛与Harness分层记录](docs/verification/2026-10-03-windows-scope-and-harness.md) |
| W04/T0无模型诊断 | [工具parser探针](docs/verification/2026-10-03-tool-parser-probe.md)，13条行为观察与独立审查通过；4d30bfa WindowsCI CTest4/4/常规Rust344/0/7通过，生产工具与完整模板接受仍未成立 |
| W04可执行窄文本验证 | [官方pi-ai验证与精确客户端锁](examples/harness/README.md)，尚非DSH/真模型/Windows通过 |
| W02开放模型 | [开放候选与验证分离](docs/verification/2026-10-03-windows-open-models.md)，50c9d41固定GGUF的WindowsCI及包发送通过，其他模型/用户目标机待验 |
| W02发现/双源下载 | [本轮验证](docs/verification/2026-10-03-model-catalog-download.md)，33f0e17 Windows363/0/7、MS固定0.6B实际下载与包复核/发送完成；用户4B下载故障另记，目标机完整验收未完成 |
| W02 aria2下载引擎 | [分层验证记录](docs/verification/2026-10-03-aria2-download-engine.md)，区分原版/策略原型/源码构建/工作树/产品结果，未宣称新引擎已交付 |
| W02下载故障诊断 | [MS重定向记录](docs/verification/2026-10-03-modelscope-redirect.md)；用户手动下载/扫描可用，具体被拒目标未知；最新CI状态见当前状态 |
| W02混合目录 | [事务与诊断验证](docs/verification/2026-10-03-mixed-model-directory.md)，本机完整workspace343 pass/7 ignored、完整clippy/UI85项及独立审查通过，8crate266项不另加总；43ad5c2 Windows344/0/7与包复核/发送完成，目标机待验，不继承旧包手验 |

历史报告是当时精确源码/设备的证据，不追溯覆盖新功能或新硬件。当前W阶段结果仍以状态与各自新报告为准。

## 可选调用层与历史

[Telegram摘要方案](docs/telegram-summary.md)是可选参考，不是runtime发布前置；源码规划路径未创建，不自动导入Telegram SDK、账号、数据库或定时任务。

Android/MNN/Flutter原计划、研究验证器、独立移动workspace与报告仅从[历史索引](docs/archive/windows-focus-2026-10-03/INDEX.md)进入，不放回当前推荐阅读顺序，不重新启动已暂停工作。
