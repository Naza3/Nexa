# Windows主线收敛与DeepSeek Harness分层验证

日期：2026-10-03。W00文档范围收敛已完成；W04最小文本互通仍进行中。本文区分文档审查、协议测试、实际客户端、真实模型与Windows目标设备，不能互相替代。

## W00：范围、提交与交付

- 分支：`codex/nexa-native-baseline`
- 文档前HEAD：`db39912ec47684ecb0325256516cafa342f16fd2`
- W00提交：[`82c4db6ecfb285e36a3aef5cfcd80cb1a1c31c91`](https://github.com/Naza3/Nexa/commit/82c4db6ecfb285e36a3aef5cfcd80cb1a1c31c91)
- Git tree：`2a158c4c4ccfe0a3392ca3e33734efed1b5c7087`
- 提交标题：`文档：聚焦Windows桌面CPU并规划DeepSeek Harness兼容`
- 已经主代理独立审查、精确提交并推送，同开发分支远端commit已核验

当前定位收敛为Windows桌面CPU本地runtime：固定llama.cpp/GGUF推理，保留Rust服务、模型、安全、单actor/队列、取消、worker和HTTP职责；Windows10 x64/i5-8400优先。当时仅做文档范围收敛；后续源码清理以[ADR0029](../decisions/0029-desktop-only-source-tree.md)为准。Telegram仅可选参考，GPU/NPU不进入当前路线。

新增[ADR0014](../decisions/0014-windows-desktop-cpu-runtime.md)和[dsh/pi-ai契约](../windows-harness-contract.md)，后续使用W00–W05。W01现包目标机短验与W04文本互通可以并行；真实工具闭环依赖W02模型工具能力，文本互通不等待W03托盘。离线、无开发工具、长期稳定性与Windows11保持后期独立验收。

## W00：实际变更与检查

本提交共23份Markdown文件：

- 10份当前文档：AGENTS、README、PROJECT_INDEX、PROJECT_STATE、执行规格，以及docs下architecture、roadmap、build-lock、model-matrix、telegram-summary
- 新增ADR0014和windows-harness-contract
- 当时创建10份旧主文档快照及INDEX；这些跨端归档现已按ADR0029从当前树移除，原始提交仍可追溯

| 检查 | 实际结果 | 边界 |
| --- | --- | --- |
| `git diff --check` | 退出0 | 文档空白/变更检查，不是业务测试 |
| Python内联Markdown检查 | 23文件、377本地路径、15锚点、代码围栏通过 | 本地目标与锚点可定位，不替代外部网站或产品运行验证 |
| 主线范围/依赖独立审查 | 通过；修正了W01/W04串行图与并行文字不一致 | 新方案不追溯修改历史通过结论 |
| 历史归档审查 | 原文/归档身份与相对链接经独立复核 | 历史文档不作为当前实施指令 |
| 提交身份 | 本地commit/tree与上述精确值一致；主代理已核验同分支远端 | 无合并、部署或公开Release |
| 构建/CI | 未运行；纯文档提交无CI运行符合预期 | 不生成新包、不授予功能或硬件支持 |

该历史文档阶段没有修改产品源码、工作流或依赖锁。后续W04代码工作属于独立切片，不计入W00。

## W04：最小文本协议切片（局部验证通过，整体进行中）

### 修改与依赖范围

本切片基于W00提交继续，局部验证、8个native-free包聚合与独立审查已通过，待后续精确提交和WindowsCI；本地全workspace因缺原生子模块未通过，详见下节。没有生产源码改动：仅`crates/runtime-api/tests/secure_transport_contract.rs`增加测试，以及[examples/harness](../../examples/harness/README.md)内8份文件（配置片段、说明、请求fixture、来源锁、官方客户端验证脚本、独立package/lock、gitignore）。没有升级引擎、改变DTO/调度/准入或放宽HTTP安全。

- 官方DSH：rc2/`639ed015`仅源码审计，未执行adapter/CLI/Web/agent
- 实际官方客户端：`@earendil-works/pi-ai@0.87.1`，OpenAI SDK`6.40.0`；全部85个依赖在独立npm lock固定，不修改根运行时依赖
- 客户端lock SHA256：`169f98129c2922511512ea2e446f178c0bdfa34cbdb5459d39a6c6dddd624330`
- 请求fixture SHA256：`5e467696e1e8ce0dddd93e6a258621e74c9411fa4f3a608ba02c959e99504fdc`
- fixture先按固定源码构造，再由实际pi-ai serializer与HTTP出站请求逐字段等值验证；不是实际DSH adapter出站golden
- 运行环境：Linux / Rust1.98.1 / Node24.19.0；真实模型、Windows设备和完整DSH均未运行

Nexa既有接口已经支持max_completion_tokens别名、include_usage及usage-only帧，本切片只补验证，不把它们算作新增生产能力。固定DSH纯文本flatten为字符串，未为pi-ai更广泛的content数组扩展DTO。工具（含空数组）、content数组/null、developer、store（含false）与未知字段保持明确拒绝。

### 实际检查与结果（实现者局部验证）

| 层级/命令 | 结果 | 证明范围 |
| --- | --- | --- |
| `cargo test --locked -p runtime-api` | 45 pass、0 fail、1 ignored | 28单元+4管理+13安全传输；ignored为需隔离npm依赖的官方客户端集成 |
| `node examples/harness/verify-pi-ai.mjs <隔离客户端目录>` | 7场景通过 | 运行官方serializer/OpenAI SDK/流解析，临时回环假服务，不是模型推理 |
| `NEXA_PI_AI_ROOT=<隔离客户端目录> cargo test --locked -p runtime-api --test secure_transport_contract harness_official_pi_ai_consumes_actual_nexa_http -- --ignored --nocapture` | 1 pass、0 fail | 官方pi-ai→Nexa真实HTTP安全层/DTO/调度/SSE→合成执行器，不是原生worker/GGUF |
| fmt与runtime-api all-targets clippy | 退出0 | 本切片格式与静态检查，不能替代根workspace聚合或Windows构建 |

7个官方客户端场景分别为stop、length、missing-finish、partial-eof、stream-error、401、404。强制1字节body分片覆盖中文/emoji UTF-8、JSON/SSE边界、空delta与usage-only；正常stop/length带有效finish和[DONE]，缺finish/半流EOF/stream error及401/404均归error。重试为0，避免掩盖首错或重复请求。

Nexa真实HTTP直连测试单独观察runtime-stop成功；合成executor只发固定协议事件，usage=3/1是测试值，不能宣称真实token计数正确。脚本仅访问精确临时回环端点、不跟随重定向、使用合成token并清理环境云凭据/代理/遥测变量，15秒硬退出，不启动工具、云模型或生产凭据初始化。

### 复现与矩阵映射

依赖准备及Windows PowerShell复现示例见[可执行验证](../../examples/harness/README.md#可执行验证)；本轮实际执行平台为Linux。独立测试客户端通过锁定npm依赖安装且禁用安装脚本，普通Rust回归不需要npm；可选集成需显式提供NEXA_PI_AI_ROOT。

已获得H02/H04/H05/H11的部分窄协议证据：认证/模型错误、实际pi-ai出站fixture、流解析与错误归类。没有将任何H编号整体标完成，H03真实Windows CPU文本、完整DSH执行、H06–H08工具协议/模型闭环、H10完整客户端阶段Stop、H12真实模型预算等仍未覆盖。

当前受控配置见[config.text-only.yaml](../../examples/harness/config.text-only.yaml)：context2048/maxTokens128、usage=true，关闭store/developer/reasoning/strict、retry0。正常DSH agent仍会发送tools，因此该片段不是能直接启动完整agent的配置，文本协议通过不等于完整CLI/Web/agent可用。

### 主代理聚合与独立审查

- `cargo test --locked --workspace`实际退出101：本地`vendor/llama.cpp`为空gitlink目录，子目录`git rev-parse`回落到父仓库，原生CMake的精确llama commit检查拒绝。失败发生在原生构建门槛，未得到全workspace通过；没有放宽锁、克隆替代引擎或用局部测试冒充全量结果
- 随后显式验证8个native-free包：runtime-types、runtime-core、model-store、runtime-ipc、process-host、runtime-api、runtime-cli、desktop-bridge。`cargo test --locked -p runtime-types -p runtime-core -p model-store -p runtime-ipc -p process-host -p runtime-api -p runtime-cli -p desktop-bridge`退出0，248 pass/0 fail/1 ignored；其中已包含API45项，不重复加总
- 同8包`cargo clippy --locked -p runtime-types -p runtime-core -p model-store -p runtime-ipc -p process-host -p runtime-api -p runtime-cli -p desktop-bridge --all-targets -- -D warnings`退出0
- 1个ignored官方客户端集成此前已显式单独执行通过；不把聚合中的ignored状态改成默认已跑
- 独立只读审查9个实现/示例文件通过，无阻断；确认无生产源码修改、严格字段/安全边界保持、临时合成loopback测试范围准确。fixture归因明确为pi-ai实际出站等值验证，非DSH adapter捕获
- 窄文本wire切片可以按此范围收口，W04整体仍进行中。最终提交身份与该精确提交的WindowsCI另行补记；CI须正常恢复锁定submodule并完成原有门禁，不能借本地native缺失跳过


## 仍未覆盖的条件

- W01新包Windows10原生目录/自动名/零复制/剪贴板等手验继续待完成
- 389eeef仍是最新已交付Windows实现；W00文档提交没有产生新二进制
- 当前精确模型准入仍仅Qwen3-0.6B Q8_0/context2048；实用模型与工具能力须W02独立准入
- 完整harness工具协议、无害真实工具回合、可选思考、全部错误/取消/预算矩阵仍未完成
- 无开发工具、实际离线、长期稳定性、Windows11及新增桌面CPU支持均不由此次文档验收获得

## 35bfd85 最终提交与Windows CI（2026-10-03 02:27 UTC）

本节补记上述窄文本协议切片的最终提交和远端回归，保留早期本地workspace失败，不将其重写为当时通过。

- 实现提交：[`35bfd85ca38d5e3c781c6b387794b265f29df18c`](https://github.com/Naza3/Nexa/commit/35bfd85ca38d5e3c781c6b387794b265f29df18c)；tree `4476d89c0c64f5008dd06a8b28e29cfb0723f2c7`
- [Windows CI37087595998](https://github.com/Naza3/Nexa/actions/runs/37087595998)于2026-10-03 02:27 UTC确认success
- 证据artifact `11261587851`；SHA256 `62713d7f1ee0306cc3c34e33877d8465e0d114472dde85d5895547a1bcffae5f`
- 主代理已下载并独立核验50项inventory、精确source/tree与逐文件hash；真实native、HTTP/CLI、Release包和desktop bridge回归通过
- `windows-rust-tests`聚合324 pass/0 fail/7 ignored；官方pi-ai可选集成仍ignored，没有在该Windows CI执行；其Linux显式单项通过证据保留在前节，不能合并称Windows客户端通过
- `native_window_tested=false`，没有新原生窗口/Windows10手工验收或完整DSH运行结论

这是旧模型门槛下35bfd85窄文本切片的Windows回归，当前[ADR0015开放模型](../decisions/0015-open-model-loading-and-validation-evidence.md)工作区代码不在其source中，不能继承通过。后续W02精确提交/新包另行验收；无须把本次测试增量包冒充新的开放模型版本。最新已实际交付用户的Windows实现仍389eeef。
