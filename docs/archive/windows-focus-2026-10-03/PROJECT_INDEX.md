> 历史快照：2026-10-03 Windows 范围收敛前的文档，仅供追溯，不是当前要求或待执行任务。原文事实未重新验收；只增加本说明并修正相对链接。当前入口见 [PROJECT_STATE.md](../../../PROJECT_STATE.md)。

# Nexa 项目索引

本文件提供定位入口，不表示规划模块已经实现。动态完成度只在 [PROJECT_STATE.md](PROJECT_STATE.md) 维护。先按任务选入口，再用 `rg` 限定范围搜索。

## 已存在的文档入口

| 入口 | 用途 |
| --- | --- |
| [README.md](README.md) | 定位、交付形态、阅读顺序 |
| [AGENTS.md](AGENTS.md) | 持续开发规则与不可破坏的边界 |
| [PROJECT_STATE.md](PROJECT_STATE.md) | 当前授权、实际状态、待决项与下一步 |
| [总体架构](docs/architecture.md) | 分层、依赖、执行流程、SDK 边界与资源归属 |
| [runtime 执行规格](ai-runtime-v0.1-execution-spec.md) | 原生集成、HTTP/IPC、默认值、T00–T10、A01–A26 |
| [Telegram 摘要方案](docs/telegram-summary.md) | 来源契约、分块、证据、任务生命周期与质量验收 |
| [开发路线](docs/roadmap.md) | runtime 与摘要两条路线、任务状态及阶段门槛 |
| [代理工作流](../../agent-workflow.md) | 角色、派发和交接格式 |
| [ADR 0001](../../decisions/0001-nexa-scope-and-layers.md) | 项目命名、优先目标、runtime 与摘要的边界 |
| [ADR0008](../../decisions/0008-android-mnn-engine-and-package.md) | Windows llama保持、Android MNN主引擎、多文件包与分后端验收 |
| [Android MNN计划](../../t07-android-mnn-plan.md) | T07-A～F版本/CPU、契约、APK、OpenCL、QNN与直接Hexagon；实际状态见验证报告 |
| [ADR0009](../../decisions/0009-android-mnn-chat-product.md) / [Android 产品计划](../../android-app-parity.md) | MNN Chat 对标、App/runtime边界、首个可用APK与后续能力 |
| [ADR0010](../../decisions/0010-model-artifact-and-conversion-provenance.md) | 公开预转换资产与自行导出两条路径，运行输入复现和转换复现分别声明 |
| [T07-A验证](../../verification/2026-10-02-t07a-mnn-cpu-probe.md) / [CPU探针](../../../native/mnn-probe/README.md) | Linux真实模型、Android原生交叉构建及未验范围 |
| [T07-B验证](../../verification/2026-10-02-t07b-mnn-runtime.md) | B1远端分阶段证据、保留失败及B2后续验收 |
| [T07-B契约](../../t07b-mnn-contract.md) / [B1 CI](../../../scripts/android_mnn/B1_CI.md) | C ABI/Rust实际接口、生产store/Executor后续门槛 |
| [MNN原生](../../../native/mnn-shim/README.md) / [Rust适配](../../../mobile/runtime/README.md) | 固定补丁、构建与真实请求验证，独立移动workspace |
| [文档基线验证](../../verification/2026-09-30-document-baseline.md) | 本轮文档检查及未验证范围 |

`.codex/config.toml` 和 `.codex/agents/*.toml` 是现有开发代理配置，不属于 Nexa 产品运行时，也不是产品依赖锁。

## 当前工程入口

已创建且可审查：`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`crates/runtime-types/`、`crates/runtime-core/`、`crates/model-store/`、`crates/engine-host/`、`crates/runtime-ipc/`、`crates/process-host/`、`crates/runtime-worker/`、`crates/runtime-api/`、`crates/runtime-cli/`、`crates/desktop-bridge/`、`apps/desktop/`、`crates/llama-adapter/`、`native/llama-shim/`、`vendor/llama.cpp/`、`native/mnn-probe/`、`native/mnn-shim/`、`native/mnn-patches/`、`mobile/runtime/`、`scripts/android_mnn/`、`xtask/`、`tests/fixtures/`。

- [构建锁](docs/build-lock.md)：固定上游/工具链、实际编译参数与限制
- [模型矩阵](docs/model-matrix.md)：真实模型与模板 hash
- [可执行验证](../../../xtask/README.md)：当前已实现命令，不混同后续 CLI
- [开发探针决策](../../decisions/0002-development-native-probe.md)：Linux 工程验证不替代目标平台门槛
- `.github/workflows/native-windows.yml`：授权开发分支的 Windows CPU 构建与真实模型 CI

- [T02核心](../../../crates/runtime-core/README.md)：单actor、队列、状态、取消、deadline、共享文本预算
- [模型存储](../../../crates/model-store/README.md)：受控导入、manifest、验证缓存和目录原子提交
- [原生执行器](../../../crates/engine-host/README.md)：专用线程和真实端到端测试
- [T02决策](../../decisions/0003-t02-scheduler-storage-and-observability.md)：存储/调度、shim2与验证边界
- [T02验证](../../verification/2026-10-01-t02-runtime.md)：逻辑测试与真实模型证据，目标平台状态
- [T03协议](../../../crates/runtime-ipc/README.md)：私有NDJSON、session/operation/seq、信用与有界codec
- [T03父端](../../../crates/process-host/README.md)：纯Rust管理进程、平台containment、故障与回收
- [T03 worker](../../../crates/runtime-worker/README.md)：独立控制/写入/原生线程、信用与真实模型验证
- [T03决策](../../decisions/0004-t03-process-isolation-and-credit-ledger.md)：独立worker、单一账本与消费lease
- [T03验证](../../verification/2026-10-01-t03-worker.md)：逐项记录逻辑/进程/真实模型/Windows边界
- [T04决策](../../decisions/0005-t04-loopback-http-and-management.md)：HTTP/CLI、分页、原子导入、服务端proof、输出预算与关停
- [T04验证](../../verification/2026-10-01-t04-http-cli.md)：Linux开发验证、固定Windows CI212项测试/真实DACL与Job、HTTP50及平台边界分开记录
- [T04 API](../../../crates/runtime-api/README.md)：本机HTTP/严格文本兼容子集；[CLI](../../../crates/runtime-cli/README.md)提供受控命令（实现/验收分别记录）

- [T05决策](../../decisions/0006-t05-windows-portable-package.md)：Windows10优先便携产品/独立验收器、真实依赖与许可、CI和目标机门槛
- [T05验证](../../verification/2026-10-01-t05-windows-package.md)：分层记录包/真实解压验收/本地设备；未执行项不算通过
- [便携包模板](../../../packaging/windows-x64-cpu/README.md)、[独立验收工具](../../../xtask/PACKAGE_ACCEPTANCE.md)：产品使用步骤和无需Cargo/Python的短验入口
- `xtask/src/windows_package.rs` / `scripts/package_windows.py`：Windows原生x64 Release构建、PE闭包/许可/hash/ZIP
- `crates/llama-adapter/native_identity.rs`：精确配置/架构/CRT/source/archive身份边界，独立rustc测试
- `scripts/stage_ci_evidence.py` / `scripts/test_stage_ci_evidence.py`：闭合允许列表脱敏证据，保留失败及合成输出hash链

- [T06契约](../../t06-desktop-contract.md)、[T06壳决策](../../decisions/0007-t06-desktop-shell-boundary.md)、[T06验证](../../verification/2026-10-01-t06-desktop.md)：固定IPC、界面与真实验收分层
- [T06外部模型目录/自动名称契约](../../t06-model-directory-contract.md)：原生只读目录选择、零复制注册、稳定ID、有效目录身份与文件保护
- `crates/desktop-bridge/`：native-free安全客户端、bounded SSE、实例/偏好/取消/关闭；`nexa-desktop-harness`验证真实模型与独立进程生命周期
- `apps/desktop/`：React/TypeScript/Vite前端与npm锁；`apps/desktop/src-tauri/`独立Rust workspace、锁、ACL和原生Windows壳
- [桌面包使用/手工UI验收](../../../packaging/desktop-windows/README.md)：与CLI包分开的完整桌面ZIP、WebView2先决条件
- `scripts/package_desktop_windows.py`、`scripts/run_desktop_smoke.py`、`scripts/test_desktop_package.py`：桌面PE/许可/hash闭包与中文空格解压真实bridge检查

## runtime 全量路径与规划

以下路径按任务逐步创建；仅上节明确列出的入口已存在，其余仍是规划。

| 规划路径 | 职责 | 对应任务 / 规格 |
| --- | --- | --- |
| `Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml` | workspace、依赖和工具链锁 | T00；规格第 3、9 节 |
| `crates/runtime-types/` | 自有 DTO、事件、错误和版本 | T01/T02；第 2、6 节 |
| `crates/runtime-core/` | actor、模型状态、队列、取消、deadline | T02；第 5–6 节 |
| `crates/model-store/` | 文件导入、manifest、原子提交 | T02；第 5 节 |
| `crates/engine-host/` | 已有PC llama原生线程执行器 | T02/T03；第 2、9 节 |
| `crates/mnn-adapter/`、`native/mnn-shim/` | Android MNN适配/C ABI规划路径，未创建 | T07；ADR0008、Android计划 |
| `crates/process-host/`、`crates/runtime-ipc/` | 父进程执行器、严格协议与信用校验 | T03；第6.4节 |
| `crates/llama-adapter/`、`native/llama-shim/` | 安全封装、C ABI、模板和原生推理 | T01；第 4 节 |
| `vendor/llama.cpp/` | 固定 commit 的上游源码 | T00；第 9 节 |
| `crates/runtime-worker/` | PC IPC、控制线程、原生线程 | T03；第 6.4 节 |
| `crates/runtime-api/`、`crates/runtime-cli/` | 本机 HTTP/SSE、鉴权和 CLI | T04；第 7–8 节 |
| `crates/runtime-mobile/` | Dart/Rust 桥与生命周期入口 | T07；第 8.3 节 |
| `apps/desktop/`、`apps/mobile/` | 桌面验证客户端；移动MNN Chat能力对标应用（移动未创建） | T06/T08；第 8 节 |
| `xtask/`、`tests/contract/`、`tests/fixtures/` | 自动构建、协议与真实推理验证入口 | T00–T09 |
| `docs/build-lock.md`、`docs/model-matrix.md` | 实测后的工具链与模型支持矩阵 | T00，之后增量维护 |
| `artifacts/verification/` | 本机详细报告，不提交私有数据 | 按相关任务生成 |

## 规划的摘要接入入口

摘要层独立于 runtime，首期放在本项目作参考接入；真实业务应用可复用或自行实现相同调用契约。下面是设计路径，尚未创建。

| 规划路径 | 职责 |
| --- | --- |
| `crates/summary-types/` | 来源快照、摘要任务、证据及产物类型 |
| `crates/summary-core/` | 分块、逐级合并、任务预算和引用校验；依赖推理客户端接口 |
| `examples/telegram-summary/` | 导出文件或已有数据的接入示例，来源方案在 S00 确定 |
| `tests/summary/` | 合成样本、回归集索引、业务质量与端到端验收 |

不预建账号服务、Telegram SDK、消息数据库或定时守护进程；只有对应接入决策确定后才增加具体实现。路线使用 S00–S04，与 runtime 的 T00–T10 分开。

## 按任务选择阅读

| 任务 | 先读 |
| --- | --- |
| 工程启动 | 状态、路线 T00、执行规格第 3、9、10 节 |
| 原生 / 模板 / 取消 | 架构执行流程、执行规格第 4、6 节、锁定版本上游头文件 |
| 调度 / 存储 / IPC | 执行规格第 5–6 节、架构资源归属 |
| API / SDK | 执行规格第 7–8 节、架构接入契约及未冻结扩展 |
| Windows UI | T06契约、ADR0007、T06验证、apps/desktop与desktop-bridge |
| Android | ADR0008/0009、Android产品计划、MNN计划、架构移动生命周期、执行规格第4.5/8.3/9.2/12.3节 |
| 摘要 | 摘要方案、路线 S00–S04；无需全文读原生实现 |
| 依赖 / 后端扩展 | build-lock、model-matrix、相关验证与决策；MNN探针构建不等于生产适配或真机通过 |
| 文档 / 交接 | AGENTS、本文、状态；按改动同步具体规范 |

## 验证入口的真实性

当前已有Cargo、原生、React前端和独立Tauri工程，已有独立Flutter设备研究验证工程apps/android-verifier，尚未完成目标手机验收。T04已新增产品CLI和xtask api-smoke；T05已有Windows专用build/独立验收器，6a7e9d0的固定Server2022 Release CI及独立Windows 10手工短验已通过，T05按当前阶段范围完成，A20无开发工具/离线与长期稳定性延后验证。T06已通过bc43e0f3的Windows CI36864041027与完整Release bridge/独立桌面包核验，另有独立手工验收确认旧包启动/导入/聊天/停止与两种关闭；最新外部目录源码75e458f的Windows CI36948947690已成功，native job110657335010含真实模型/runtime/HTTP/CLI、桌面包和解压bridge验收通过；下载产物独立复核通过并已交付，新目录原生UI仍未测，详见[T06最终目录版记录](../../verification/2026-10-01-t06-desktop.md#第七轮目录版windows-ci成功独立复核与交付2026-10-02)。执行规格第12节的通用check、contract套件与adb install仍按阶段推进，当前可运行范围见xtask/README.md、API/CLI README和状态文件。

- 文档：检查相对文件链接、围栏、旧项目残留和内容一致性；有 Git 时执行 `git diff --check`。
- 工程建立后：按执行规格第 12 节和实际脚本执行定向检查，在状态和验证记录写退出码。
- 真实推理：必须记录模型 hash、模板、后端、设备和输入版本；未测不填零或通过。
- 摘要：必须核对覆盖、引用、事实归因和整份任务耗时，文本非空不等于质量合格。


## Android 设备研究验证新增入口

- [B3a 验证域与 JNI/FD 边界](../../decisions/0011-android-device-verifier-domain.md)
- [设备验证器计划](../../t07c-android-verifier-plan.md)
- [CI 研究证据收据决策](../../decisions/0012-ci-research-evidence-receipts.md)
- [设备研究 App](../../../apps/android-verifier/README.md)：固定 CPU 验证、导入与报告；不是完整聊天产品
- [一加15首轮安装与模型/报告验收](../../android-device-verifier-acceptance.md)
- [c0c0927最终研究APK与Android CI交付记录](../../verification/2026-10-02-android-device-verifier-delivery.md)

- [共享core取消与真实故障修正](../../verification/2026-10-02-core-cancellation-faults.md)：普通取消不掩盖真实错误、shutdown当次故障保留及226项控制回归

- [当前Windows重点与Android路线调整](../../decisions/0013-windows-focus-and-android-mnn-chat.md)：独立Android App暂停，保留研究结果

- [Windows模型兼容性与API示例验证](../../verification/2026-10-02-windows-model-compatibility.md)：准入原因、旧客户端兼容、144项Rust与74项前端回归
