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
| [代理工作流](docs/agent-workflow.md) | 角色、派发和交接格式 |
| [ADR 0001](docs/decisions/0001-nexa-scope-and-layers.md) | 项目命名、优先目标、runtime 与摘要的边界 |
| [文档基线验证](docs/verification/2026-09-30-document-baseline.md) | 本轮文档检查及未验证范围 |

`.codex/config.toml` 和 `.codex/agents/*.toml` 是现有开发代理配置，不属于 Nexa 产品运行时，也不是产品依赖锁。

## 规划的 runtime 入口

以下路径在工程任务中逐步创建，当前不可当作现有代码或可运行命令。

| 规划路径 | 职责 | 对应任务 / 规格 |
| --- | --- | --- |
| `Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml` | workspace、依赖和工具链锁 | T00；规格第 3、9 节 |
| `crates/runtime-types/` | 自有 DTO、事件、错误和版本 | T01/T02；第 2、6 节 |
| `crates/runtime-core/` | actor、模型状态、队列、取消、deadline | T02；第 5–6 节 |
| `crates/model-store/` | 文件导入、manifest、原子提交 | T02；第 5 节 |
| `crates/engine-host/` | process / embedded 执行器 | T02/T03/T07；第 2、9 节 |
| `crates/llama-adapter/`、`native/llama-shim/` | 安全封装、C ABI、模板和原生推理 | T01；第 4 节 |
| `vendor/llama.cpp/` | 固定 commit 的上游源码 | T00；第 9 节 |
| `crates/runtime-worker/` | PC IPC、控制线程、原生线程 | T03；第 6.4 节 |
| `crates/runtime-api/`、`crates/runtime-cli/` | 本机 HTTP/SSE、鉴权和 CLI | T04；第 7–8 节 |
| `crates/runtime-mobile/` | Dart/Rust 桥与生命周期入口 | T07；第 8.3 节 |
| `apps/desktop/`、`apps/mobile/` | 最小模型管理及推理验证 UI | T06/T08；第 8 节 |
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
| Android | 架构移动生命周期、执行规格第 8.3、9.2、12.3 节 |
| 摘要 | 摘要方案、路线 S00–S04；无需全文读原生实现 |
| 依赖 / 后端扩展 | build-lock、model-matrix（创建后）、相关验证与决策 |
| 文档 / 交接 | AGENTS、本文、状态；按改动同步具体规范 |

## 验证入口的真实性

当前没有 Cargo、前端或 Flutter 构建入口。执行规格第 12 节中的 `cargo run -p xtask ...`、CLI 和 `adb install` 是未来命令契约，先检查实现和前置条件，再执行。

- 文档：检查相对文件链接、围栏、旧项目残留和内容一致性；有 Git 时执行 `git diff --check`。
- 工程建立后：按执行规格第 12 节和实际脚本执行定向检查，在状态和验证记录写退出码。
- 真实推理：必须记录模型 hash、模板、后端、设备和输入版本；未测不填零或通过。
- 摘要：必须核对覆盖、引用、事实归因和整份任务耗时，文本非空不等于质量合格。
