# Nexa 当前状态

最后更新：2026-10-03。工程实现、逻辑测试、真实模型、Windows CI、原生窗口、目标设备与后期发行条件分层记录。唯一当前排期见本文件；详细历史见 [归档索引](docs/archive/windows-focus-2026-10-03/INDEX.md)。

## 当前目标与授权

按 [ADR0014](docs/decisions/0014-windows-desktop-cpu-runtime.md)，Nexa 聚焦 Windows 桌面 CPU 本地 LLM runtime，以 llama.cpp/GGUF 为核心，通过 API 供其他应用调用。Windows10 x64 / i5-8400 / 16GB内存为首要目标，后续按实测扩大 Intel/AMD 桌面 CPU 与 Windows11；桌面 UI 逐步完善为管理器，聊天为辅助。

用户明确要求API兼容官方DeepSeek Harness（dsh）；只读研究已确认rc2基线与pi-ai自定义openai-completions路线，见[harness契约](docs/windows-harness-contract.md)。官方pi-ai的受控文本协议子集已有实测；DSH本体、工具协议/实用模型/真实agent回合未执行或验证，不能把文本子集宣称为完整兼容。用户2026-10-03已明确“可以，现在逐步推进”，后续Windows路线实施已获授权。已完成W00为纯文档；后续W02源码/原生构建与验证独立记录；W04最小文本互通可独立推进，不等待W01目标机窗口或W03托盘。既有开发分支/CI授权不扩张为独立项目、新权限、合并或部署。

Android 设计退出当前主线；历史研究源码、报告、隔离 CI 与 B3b 未提交 WIP 保留。外部 MNN Chat fork 是独立项目，不改。Telegram 摘要为可选参考调用端，不构成 runtime 发布依赖。无开发工具、实际离线和长期稳定性仍列后期验收。

用户补充目标机16GB，要求支持很多模型而非仅特定几个。按[ADR0015](docs/decisions/0015-open-model-loading-and-validation-evidence.md)开放符合结构/安全/文本契约的候选尝试；validated仅保存历史证据，不作模型名/hash白名单。开放实现及闭包修正已提交`50c9d41`，最终WindowsCI于2026-10-03 06:50 UTC成功，原固定GGUF的真实模板/推理与完整包链路已回归；独立桌面包字节闭包复核通过，原字节包已于07:03 UTC发送，消息发送获接受；用户下载或运行尚未确认。35bfd85与已交付389eeef不可追溯获得新行为，其他模型与Win10目标机仍未因此验收。

当前W02混合目录切片已提交`43ad5c2`，实施基线为`f3e1b90`；按[ADR0016](docs/decisions/0016-mixed-model-directory-diagnostics.md)增加合法集合一次原子partial提交、完整有界诊断和仅扫描短context默认值。最终主代理全workspace聚合343 pass/0 fail/7 ignored、完整clippy和UI85项/typecheck/lint/build通过（写入者8crate266/0/1为其中子集，不累加），独立源码/事务审查无阻断；精确提交WindowsCI37108375458已success，50项证据/source/大小/hash已核，混合扫描/被拒文件guard、固定GGUF与完整包/bridge通过；独立下载包复核通过，原字节43ad5c2包于08:45:45 UTC发送获接受，用户下载/运行尚未确认。旧50c9d41包仍是整批失败行为，详见[本轮记录](docs/verification/2026-10-03-mixed-model-directory.md)。

## 已有工程与最新交付

| 范围 | 状态与证据 |
| --- | --- |
| 检查基线 | `codex/nexa-native-baseline`；当前源码`43ad5c27333c2c493bba7c14fbfbcf76288d18b0`，tree`72f5932f7f555246d2f8890acc34b165fee26b5f`，混合目录切片WindowsCI通过；最新已发送Windows实现为43ad5c2，尚未确认用户下载或运行 |
| 推理核心 | llama.cpp固定`2149c00f4442dc59302e134a02e4c99d5f7ed9fc`；C++ shim、模板/token预算/采样、UTF-8/stop、取消/释放已有真实回归 |
| T00–T04 | 固定 Windows CPU 的原生链、model-store、单actor/队列、独立worker/IPC/Job、HTTP/CLI阶段已完成；详情见[索引](PROJECT_INDEX.md) |
| T05 | Release便携包/独立工具、PE/依赖/许可/hash及独立Windows10短验已按阶段范围收口；A19/A20后期条件未完成 |
| T06 | 目录选择、零复制、自动名、兼容原因、参数设置、聊天/停止、服务启停/两种关闭已实现；新包原生UI剩余分支待验 |
| 模型加载/证据 | 旧交付389eeef仅开放固定0.6B；50c9d41已实现独立loadable与历史validated，移除模型名/hash许可名单并保留安全/模板/预算门槛；此次CI真实模型仍仅固定Qwen3-0.6B Q8_0/context2048，其他候选未标已实测 |
| 当前版本CI | [Windows37108375458](https://github.com/Naza3/Nexa/actions/runs/37108375458)成功，job111161243743；48组344 pass/0 fail/7 ignored、CTest3/3、external17（含被拒文件guard）及固定真实模型/store/core/worker/HTTP/CLI/完整包/解压bridge通过；50项证据身份/hash已核；`native_window_tested=false` |
| 最新交付 | `Nexa-Windows-x64-43ad5c2.zip`，9,432,048 bytes，SHA256`5dc8cffe0fd4113b715a989566d481f5ff482099327d036e4768c2af7d66f7b5`；原ZIP字节未改；750文件/runtime197、6个AMD64 PE的普通及delay imports、许可540+6+186项独立核验通过；3 CRT与CI微软签名记录一致，Linux未重新Authenticode验签；2026-10-03 08:45:45 UTC发送获接受 |
| 上一交付 | `Nexa-Windows-x64-50c9d41.zip`，9,423,216 bytes，SHA256`713cd39d78adeb38e585529f3e188c9a3912090651172e3b268fb21bcab5c47f`；原ZIP内容未改；750文件/嵌套runtime197文件、6个AMD64 PE导入闭包及许可540+6+186项记录独立复核通过；3个CRT与CI微软签名记录一致，Linux未重新签名或验签；2026-10-03 07:03 UTC消息发送获接受 |
| 历史交付 | `Nexa-Windows-x64-389eeef.zip`，9,789,508 bytes；SHA256 `45251f28c2eb61a1b6ee5119aab3b0923a8117c677fef4ec91ea680be1b209f0`；750文件、嵌套runtime、6个PE与许可hash已独立复核；2026-10-02 13:36 UTC附件发送被接受 |

旧389eeef交付据[历史记录](docs/verification/2026-10-02-windows-model-compatibility.md#最终提交ci与交付)；50c9d41历史CI、包复核与交付范围见[开放模型记录](docs/verification/2026-10-03-windows-open-models.md#最终50c9d41-windows-ci与交付产物2026-10-03)。43ad5c2最新WindowsCI/包复核/交付见[混合目录记录](docs/verification/2026-10-03-mixed-model-directory.md#精确43ad5c2-windowsci与交付产物)。文档更新不表示用户已下载或运行。旧包 `bc43e0f3` 已有原生启动、导入、聊天、停止和两种关闭手验；不能追溯证明新目录版窗口操作通过。

## 当前路线状态

| 阶段 | 状态 | 下一步/边界 |
| --- | --- | --- |
| W00 主线收敛 | 已完成 | `82c4db6`独立审查、文档/归档检查与远端身份核验通过；纯文档无源码改动、无CI运行，见[本轮记录](docs/verification/2026-10-03-windows-scope-and-harness.md) |
| W01 当前版本短验 | 待验证 | 已发送43ad5c2新包，待验Windows10混合目录诊断/全坏保旧/空目录清空、零复制/自动名、开放候选提示、剪贴板及独立API；CI bridge不替代窗口手验，等待用户目标机窗口 |
| W02 开放模型与CPU性能 | 进行中 | 用户要求16GB机器广泛模型支持；50c9d41完整WindowsCI及固定GGUF真实回归通过；独立包复核与发送完成；用户Win10/i5-8400/16GB验收与其他模型/性能仍待完成；本轮43ad5c2混合目录增量已过本机/独立审查及精确WindowsCI，独立产物复核及发送完成，用户目标机仍待验；基准样本不是产品名单，见[开放模型记录](docs/verification/2026-10-03-windows-open-models.md) |
| W03 桌面管理器 | 未开始 | 托盘/窗口恢复与API诊断体验；当前已有服务启停和关窗保留服务 |
| W04 API / deepseek harness | 进行中 | 窄文本协议切片完成：pi-ai7场景、真实HTTP+合成执行器1项、8个native-free包248回归及clippy/独立审查通过；早期本地全workspace因缺子模块失败；35bfd85 WindowsCI324/0/7及旧模型真实链已通过，DSH/Windows pi-ai/工具未跑，见[分层记录](docs/verification/2026-10-03-windows-scope-and-harness.md) |
| W05 后期发行验收 | 未开始 | 无开发工具、实际离线、长期稳定性、升级/回退、Windows11及完整支持矩阵 |

各阶段最小增量、依赖与验收见 [路线](docs/roadmap.md)。目标机验收暂不可执行时，可推进W04已授权的最小文本互通切片、W02验证准备，不降低验收门槛。

## 实现与验证限制

- 当前 Windows CI 主要证据来自 Server2022/EPYC/2逻辑CPU，不能推广成 i5-8400 或任意 Intel/AMD 支持；历史4线程超配探针60秒超时完整保留
- 默认API配置context4096/batch512，桌面验证档2048/2线程/128；历史真实证据仅覆盖其精确组合。开放切片的模型metadata/131072硬限不代表16GB可运行该窗口；无新增Job RAM硬限，不宣传OOM绝对隔离
- 接口现为严格文本 Chat Completions 子集，不包含已验证的工具调用、结构化输出或完整 harness 兼容性
- `status/devices` 未知 native 指标为 null/unavailable；配置值不伪装成实测值
- Windows worker清理未获OS确认时fail-closed，不假称已回收；已有跨层取消/真实故障优先级修复保留
- ASan/UBSan纯流缓冲测试不是全原生库无泄漏证明；长期100请求/20加载趋势仍后期验收
- 当前无托盘、开机自动启动、完整聊天持久化或新硬件加速的实现承诺

## 最小下一步

1. W00已完成；W04窄文本切片已提交35bfd85，其[WindowsCI37087595998](https://github.com/Naza3/Nexa/actions/runs/37087595998)已于02:27 UTC成功，50项证据/身份/hash核验通过；只覆盖35bfd85，不覆盖本次开放模型工作区变更
2. 43ad5c2桌面包独立字节闭包复核及发送完成；待用户下载/运行后做W01 Win10短验，消息发送获接受不等于已运行
3. W02混合目录43ad5c2已通过WindowsCI、包内验收与独立下载包复核，原字节包已发送；等待用户目标机验收，按[本轮矩阵](docs/verification/2026-10-03-mixed-model-directory.md)逐层记录；旧50c9d41 CI不覆盖该增量。继续其他模型/目标16GB机实测，不扩大已验证矩阵。补W04完整DSH/真实模型文本与独立工具能力；pi-ai fixture仍非DSH本体捕获
4. 在实测或明确协议基础上选下一块最小实现，不重复已完成能力；不恢复 Android 或绑定 Telegram 业务

## 历史与保留工作

完整旧状态原文（含T00–T08、Android研究及历次失败/交付）见 [状态快照](docs/archive/windows-focus-2026-10-03/PROJECT_STATE.md)。既有验证报告仍原位保存。`apps/android-verifier/` 14项修改/未跟踪文件是暂停的B3b WIP，不属于本轮；不得删除、暂存或覆盖。它不构成可交付的新APK或生产支持。


### 开放模型提交与构建状态

8522514的Windows37101303658与历史MNN37101303651已cancelled，不记为失败推理或通过。闭包修正50c9d41的Windows37101760025已success，当前产物独立字节闭包复核已通过，原字节包于07:03 UTC发送获接受；目标机下载/运行仍待确认。50c9d41的历史MNN研究回归37101760095另已成功，只用于共享DTO迁移回归，不是Android App/设备验收，未恢复Android产品线；旧B3b WIP未动。详见[最终CI记录](docs/verification/2026-10-03-windows-open-models.md#最终50c9d41-windows-ci与交付产物2026-10-03)。
