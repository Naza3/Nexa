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
| [模型使用流程ADR0019](docs/decisions/0019-model-onboarding-and-local-validation.md) | 自动登记、服务关闭时浏览、本机加载/短生成证据与不抢占的自动动作；实施/验收状态见当前状态 |
| [可选局域网API ADR0020](docs/decisions/0020-opt-in-lan-inference-api.md) | 默认关闭的独立LAN推理监听、私有范围/独立Bearer、已加载模型原子准入和本机管理隔离；实施与验证中 |
| [局域网使用说明](docs/lan-api-usage.md) | 可信私网配置、Base URL/密钥、已加载模型和目标机验收步骤；不自动修改防火墙 |
| [选中文件添加ADR0021](docs/decisions/0021-selected-file-model-registration.md) | 原生单/多文件零复制定向登记，schema2跨目录来源、默认不自动全扫；实施中 |
| [本机证明修复ADR0022](docs/decisions/0022-windows-local-validation-paths-and-feedback.md) | Windows内部canonical数据路径、外部路径边界保持、本次测试反馈与历史证明分离；源码验证完成、Windows待验 |
| [超时与空闲策略ADR0023](docs/decisions/0023-model-verification-and-idle-policy.md) | 独立的整操作文件校验deadline、显式不自动卸载、停服安全保存；源码回归完成，Windows待验 |
| [工具契约草案](docs/windows-tools-contract.md) | 生产tools尚未实现；T0已有无模型parser观察并定位严格完整性/schema/普通文本缺口 |
| [构建锁](docs/build-lock.md) / [模型矩阵](docs/model-matrix.md) | 固定工具链/llama与精确模型验证证据，非运行许可名单 |
| [代理协作](docs/agent-workflow.md) | 单写入者、检查和交接 |
| [桌面源码边界 ADR0029](docs/decisions/0029-desktop-only-source-tree.md) | 安装器交付后移除本项目移动实现/专用文档，Git 历史与独立项目不变 |

- [模型移除 ADR0030](docs/decisions/0030-nondestructive-model-unregistration.md)：仅取消登记、文件保留、schema3原子可见性及在线/离线互斥
- [聊天 Markdown ADR0031](docs/decisions/0031-safe-chat-markdown.md)：安全GFM显示、原文保持与离线链接/图片边界

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

根 workspace、桌面壳 workspace 与相关 Windows/Linux 开发验证是当前工程入口。`.codex/agents/`是开发代理配置，不是产品运行依赖；上游 llama.cpp 的跨平台源码保持完整。

## Windows 接口、发行与原生窗口

- [Tag 自动发行](docs/windows-releases.md)：同一精确源码/payload 的便携 ZIP、MSI、Setup EXE、版本与对应源码闭包；分支构建不发布，平台/目标机验收分层
- [一键版本更新](update-version.cmd) / [跨平台工具](scripts/set_version.py)：离线同步七个版本文件，支持预览与一致性检查；[本轮验证](docs/verification/2026-10-07-release-version-tool.md)
- [公网 TLS 检查有限重试](docs/verification/2026-10-07-public-tls-retries.md)：只重试已识别网络故障，保留全部尝试证据与原通过门槛
- [Windows 安装器](docs/windows-installers.md) / [ADR0028](docs/decisions/0028-tagged-windows-installers.md)：当前用户 MSI 与同载荷 Setup、数据保留、生命周期与目标机验收边界

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

## 可选调用层与后续验证

[Telegram摘要方案](docs/telegram-summary.md)是可选参考，不是runtime发布前置；源码规划路径未创建，不自动导入Telegram SDK、账号、数据库或定时任务。

当前源码树不保留项目自有移动工程、专用设计和研究报告。删除范围与开发验证见[桌面清理记录](docs/verification/2026-10-05-desktop-only-cleanup.md)，历史内容可由 Git 追溯。

- [Windows本机测试记录修复验证](docs/verification/2026-10-04-windows-model-evidence.md)

- [运行策略与最终联合回归](docs/verification/2026-10-04-runtime-policy-and-final-regression.md) / [Windows联合验收步骤](docs/verification/2026-10-04-windows-model-management-checklist.md)

- [公开仓库恢复Windows Actions](docs/verification/2026-10-04-public-actions-restoration.md)：用户最新构建授权、标准runner、同源工具链与推送前验证

- [桌面控制与网卡选择验证](docs/verification/2026-10-04-desktop-controls-and-lan-discovery.md)：添加结果关闭、统一服务主控与只读本机IPv4发现；实现/验证进展见报告

- [空ID默认当前模型ADR0024](docs/decisions/0024-current-loaded-model-chat-default.md)：用户确认的文本API便利扩展，actor原子选择、不隐式加载/切换，本地源码验证通过、统一Windows构建待验

- [空ID当前模型验证](docs/verification/2026-10-04-current-model-api-default.md)：DTO、actor原子准入和SSE/非流式实际模型身份，Windows新包待验

- [整体产品体验实施](docs/product/experience-implementation.md)：用户批准的状态、配置、模型档案、任务和页面统一规则，当前实施中

- [统一档案与配置ADR0025](docs/decisions/0025-unified-model-profiles-and-configuration.md)：schema2、CAS迁移、共享解析、会话快照和前后端命令合同，实施中

- [整体体验实施验证](docs/verification/2026-10-04-unified-product-experience.md)：档案/CAS、状态、任务、独立反例与真实GGUF分层证据

- [许可无损整合ADR0026](docs/decisions/0026-lossless-license-bundles.md)：完整桌面含runtime/download至多10份许可文件，保留全部原文、版权HTML、Microsoft原件与原库存映射

- [许可整合验证](docs/verification/2026-10-05-lossless-license-bundles.md)：Python/Rust共享合同、77项针对性Rust回归、748/760份原文旧包语料逐字节恢复和未验证的Windows边界

- [精简桌面产品体验](docs/product/compact-desktop-experience.md)：全局状态条、分组摘要和按需展开设置的整体设计

- [本轮精简桌面验证](docs/verification/2026-10-05-compact-desktop-experience.md)：前端回归、独立审查与原生Windows验证边界

- [手动停止加载ADR0027](docs/decisions/0027-owned-model-load-cancellation.md) / [本轮验证](docs/verification/2026-10-05-model-load-cancellation.md)：按次UUID、hash/切换/native/probe、回执恢复与worker回收；状态以当前状态为准

- [Tag 发行开发验证](docs/verification/2026-10-05-tag-release.md)：版本、封闭资产、精确 tag/commit 与无覆盖发布反例；Windows 安装器结果分层记录

- [LAN保存CI时序修复](docs/verification/2026-10-05-lan-save-ci-race.md)：mock持久状态与旧读取微任务竞争的确定性对照，安装器CI重跑前置

- [本机单图OCR ADR0032](docs/decisions/0032-local-single-image-ocr.md)：配对GGUF、CPU mtmd、图片HTTP/IPC预算与桌面流程
- [Windows CPU OCR使用说明](docs/ocr-windows-cpu.md)：双文件下载、显式加载、图片/Markdown及i5-8400参数建议
- [单图OCR开发验证](docs/verification/2026-10-07-local-ocr.md)：Linux真实模型/HTTP及文本回归，Windows与目标机另验
