# Nexa 当前状态

## 2026-10-04 当前覆盖与最小下一步

本节是本轮最新状态，覆盖下文截至 2026-10-03 的历史“当前”安排、旧 CI 授权和最小下一步；保留原有历史验证事实，不重写旧提交或把旧证据转授新版本。

- 当前任务：在 `0d5b1dd5e77807239d8af99d39755ee381b2fae9` aria2 整合基线上修复 Windows 打包器的 VS 选择，优先复用用户已安装的 VS2026；已有 VS2022 也可复用，无可用环境时才给 VS2022 Build Tools 兜底指引。修复包含生成器/同实例 MSVC与CRT/原生和Cargo缓存隔离；35项Python逻辑检查为33通过/2 Windows专用跳过，独立源码审查无阻断，原生Windows状态为待验证，见[构建锁](docs/build-lock.md#2026-10-04-当前覆盖复用既有-visual-studio本地-windows-手动构建)和[本轮记录](docs/verification/2026-10-04-windows-vs-selection.md)
- 构建约束：以后不在 GitHub Actions 构建本项目 Rust，改为用户手动本地 Windows 构建。用户当前不能连接电脑；没有目标 Windows 执行证据，不把 Python 单测当成 VS2026 构建通过。本轮未运行或触发 CI
- 交付区分：最新已发送完整 App 仍为 `33f0e17`；`0d5b1dd` 源码及独立预编译 aria2 组件已另行提供，只是本地构建输入。含本次修复的新完整 App 尚未构建/交付。新提交须由交付方真实重建同提交 aria2 并复核来源闭包，不能改清单冒充；不要求用户编译 aria2
- 工具链：Rust `1.98.1`、CMake `4.4.3` 与固定 llama.cpp 不变。用户已安装 stable MSVC 别名；目标工程的实际 `rustc -vV` release/host 仍须核对。前端后续统一 pnpm，本次明确暂缓，保留现有锁/命令，不中途重装迁移
- 下一步：本次脚本逻辑回归与独立源码审查已完成；形成新提交时重新提供真实同源 aria2 组件，再由用户在本地完成 runtime、桌面及完整包验证。记录实际 VS/MSVC/SDK 身份、启动/下载/取消/独立 size 与 SHA、显式扫描/加载结果；Windows10/i5-8400/16GB 与其他模型的历史待验项不降低

### 以下保留 2026-10-03 状态与历史证据

最后更新：2026-10-03。工程实现、逻辑测试、真实模型、Windows CI、原生窗口、目标设备与后期发行条件分层记录。唯一当前排期见本文件；详细历史见 [归档索引](docs/archive/windows-focus-2026-10-03/INDEX.md)。

## 当前目标与授权

按 [ADR0014](docs/decisions/0014-windows-desktop-cpu-runtime.md)，Nexa 聚焦 Windows 桌面 CPU 本地 LLM runtime，以 llama.cpp/GGUF 为核心，通过 API 供其他应用调用。Windows10 x64 / i5-8400 / 16GB内存为首要目标，后续按实测扩大 Intel/AMD 桌面 CPU 与 Windows11；桌面 UI 逐步完善为管理器，聊天为辅助。

用户明确要求API兼容官方DeepSeek Harness（dsh）；只读研究已确认rc2基线与pi-ai自定义openai-completions路线，见[harness契约](docs/windows-harness-contract.md)。官方pi-ai的受控文本协议子集已有实测；DSH本体、工具协议/实用模型/真实agent回合未执行或验证，不能把文本子集宣称为完整兼容。用户2026-10-03已明确“可以，现在逐步推进”，后续Windows路线实施已获授权。已完成W00为纯文档；后续W02源码/原生构建与验证独立记录；W04最小文本互通可独立推进，不等待W01目标机窗口或W03托盘。既有开发分支/CI授权不扩张为独立项目、新权限、合并或部署。

Android 设计退出当前主线；历史研究源码、报告、隔离 CI 与 B3b 未提交 WIP 保留。外部 MNN Chat fork 是独立项目，不改。Telegram 摘要为可选参考调用端，不构成 runtime 发布依赖。无开发工具、实际离线和长期稳定性仍列后期验收。

用户补充目标机16GB，要求支持很多模型而非仅特定几个。按[ADR0015](docs/decisions/0015-open-model-loading-and-validation-evidence.md)开放符合结构/安全/文本契约的候选尝试；validated仅保存历史证据，不作模型名/hash白名单。开放实现及闭包修正已提交`50c9d41`，最终WindowsCI于2026-10-03 06:50 UTC成功，原固定GGUF的真实模板/推理与完整包链路已回归；独立桌面包字节闭包复核通过，原字节包已于07:03 UTC发送，消息发送获接受；用户下载或运行尚未确认。35bfd85与已交付389eeef不可追溯获得新行为，其他模型与Win10目标机仍未因此验收。

当前W02混合目录切片已提交`43ad5c2`，实施基线为`f3e1b90`；按[ADR0016](docs/decisions/0016-mixed-model-directory-diagnostics.md)增加合法集合一次原子partial提交、完整有界诊断和仅扫描短context默认值。最终主代理全workspace聚合343 pass/0 fail/7 ignored、完整clippy和UI85项/typecheck/lint/build通过（写入者8crate266/0/1为其中子集，不累加），独立源码/事务审查无阻断；精确提交WindowsCI37108375458已success，50项证据/source/大小/hash已核，混合扫描/被拒文件guard、固定GGUF与完整包/bridge通过；独立下载包复核通过，原字节43ad5c2包于08:45:45 UTC发送获接受，用户下载/运行尚未确认。旧50c9d41包仍是整批失败行为，详见[本轮记录](docs/verification/2026-10-03-mixed-model-directory.md)。

当前W04/T0进行无模型工具parser证据实验：13条锁定上游模板/parser观察、Release CTest4/4、主代理复验与独立审查通过；发现final LENIENT可接受不完整调用、strict全匹配不验证schema/调用数且普通文本分支不成立的具体缺口。尚无完整工具/文本接受算法，生产API/tools/版本均未改，其精确4d30bfa WindowsCI37115797798现已success（CTest4/4、常规Rust344/0/7、50报告hash已核），无模型工具实验和真实DSH/工具能力结论仍分开；未另发T0二进制，也不覆盖新目录下载增量，见[T0记录](docs/verification/2026-10-03-tool-parser-probe.md)。

当前优先事项按用户2026-10-03 10:47 UTC最新要求恢复模型加载流程：修复无配置时EXE/models自动发现，增加默认ModelScope/HF可选的固定8条目录下载，保存后显式扫描/加载。见[ADR0017](docs/decisions/0017-model-discovery-and-catalog-download.md)与[本轮记录](docs/verification/2026-10-03-model-catalog-download.md)。本机完整聚合与独立源码审查通过；前三次WindowsCI的构建预算、preview超时、路径显示断言失败及修正均保留。最终33f0e17的WindowsCI37124146573成功，52项证据身份/大小/hash已核，常规Rust49组363/0/7、CTest4/4通过。产品下载器经默认MS实际下载固定0.6B Q8_0共639,446,688字节，完整hash符合基线，47,149ms后发布且registered=false；后续独立基线、真实推理/停止/core/worker/HTTP/CLI、Release包与解压bridge链路通过。原生窗口未执行，HF实际下载、其他7个模型和Win10/i5-8400/16GB仍待验。新包独立字节闭包审查通过，原字节33f0e17包于13:42:49 UTC发送获接受；交付当时下载/运行未确认，后续用户下载流程反馈见下文。Harness新实施继续暂停。

当前用户反馈：Qwen3-4B-Q4_K_M经ModelScope下载时0B立即失败，诊断码为`model_download_redirect_rejected`；同一链接在浏览器可用，用户随后确认手动下载后扫描可以。该确认不包含加载、聊天或性能；具体被拒目标仍未知，不能归因为目录权限。诊断与探针已提交5266ab6，本机bridge79/UI119、Python97项（95通过/2平台skip）及独立审查通过；[Windows诊断CI37132080750](https://github.com/Naza3/Nexa/actions/runs/37132080750)已成功（产物待复核），[有界路由观察37132080792](https://github.com/Naza3/Nexa/actions/runs/37132080792)成功记录0.6B与4B均为MS200、无重定向、各4096字节GGUF前缀，只是该CI路径观察，不能复现或解释用户被拒host。见[排查记录](docs/verification/2026-10-03-modelscope-redirect.md)。

用户要求通用下载引擎并允许开源组件，已采纳aria2 1.37.0受控sidecar，不再自建HTTP/Range。主线8c82203上的工作树已实现监督器、model-store事务、bridge/壳组件身份、原生下载验证器与打包/CI；最终Windows/真实MS/HF及新包未完成，旧具体被拒host仍未知。主代理在崩溃残留layout收尾前已完成全workspace all-targets40组389/0/7及clippy/格式、UI149项/typecheck/lint/build、Python126项（124通过/2平台skip）；其后最终残留规则追加壳28项/clippy/格式及独立补审通过，desktop11为Python126子集不累加；未重跑完整workspace，局部engine20/store11/bridge79也不重复相加。辅助源码构建01db921的[CI37138664930](https://github.com/Naza3/Nexa/actions/runs/37138664930)已过Linux构建/fixture；Windows首次job111249100173及17:05重跑的job111249990877均在runner_id=0、steps为空时失败，代码未执行；启动原因未知，17:10已请用户提供run顶部错误，等待证据而不第三次盲重跑。最终集成Windows验收受阻；本轮整合使用待验检查点分支`codex/nexa-aria2-integration`保存，不更新主开发分支、不触发其CI或表示发布通过。旧实现8c82203的[CI37135712318](https://github.com/Naza3/Nexa/actions/runs/37135712318)已成功，52份证据独立核对通过（Rust368/0/7、CTest4/4、旧下载器MS固定0.6B成功），不转授本轮aria2实现。

首版保持无RPC/固定argv/env、受核验的download/nexa-aria2.exe、任务内恢复及仅exit8的一次全量restart，attempt1→2共享deadline。父端独立size/SHA、no-clobber与取消CAS决定发布。网络gate仅约束initial/redirect/aria2实际下载socket，SChannel自动证书/吊销与OS AIA/CRL/OCSP仍是独立平台边界；跨App重启恢复/代理不属于首片。见[ADR0018](docs/decisions/0018-generic-download-engine-candidate.md)与[新验证记录](docs/verification/2026-10-03-aria2-download-engine.md)。新引擎尚未交付，Harness新实施继续暂停。

## 已有工程与最新交付

| 范围 | 状态与证据 |
| --- | --- |
| 检查基线 | 主线`8c82203c1ff2f73981575733bd81a88c6cfd4f8a`；aria2整合为`codex/nexa-aria2-integration`待验检查点，辅助源码分支01db921的Windows启动受阻，旧实现主线CI已通过，最终新引擎待验；最新已发送仍33f0e17 |
| 推理核心 | llama.cpp固定`2149c00f4442dc59302e134a02e4c99d5f7ed9fc`；C++ shim、模板/token预算/采样、UTF-8/stop、取消/释放已有真实回归 |
| T00–T04 | 固定 Windows CPU 的原生链、model-store、单actor/队列、独立worker/IPC/Job、HTTP/CLI阶段已完成；详情见[索引](PROJECT_INDEX.md) |
| T05 | Release便携包/独立工具、PE/依赖/许可/hash及独立Windows10短验已按阶段范围收口；A19/A20后期条件未完成 |
| T06 | 目录选择、零复制、自动名、兼容原因、参数设置、聊天/停止、服务启停/两种关闭已实现；新包原生UI剩余分支待验 |
| 模型加载/证据 | 旧交付389eeef仅开放固定0.6B；50c9d41已实现独立loadable与历史validated，移除模型名/hash许可名单并保留安全/模板/预算门槛；此次CI真实模型仍仅固定Qwen3-0.6B Q8_0/context2048，其他候选未标已实测 |
| 最新交付版本CI | 33f0e17 / job111205956541；Windows常规Rust363 pass/0 fail/7 ignored、CTest4/4，默认MS固定0.6B真实下载及后续独立hash/真实推理/完整包/bridge通过；新桌面ZIP11,119,286 bytes、SHA256`b37d89cbd1baf1dfd07d7dbdeae794157504d4c5a4064e171e7c4851d1015c1b`；`native_window_tested=false`，独立下载包审查通过 |
| 上一交付版本CI | [Windows37108375458](https://github.com/Naza3/Nexa/actions/runs/37108375458)成功，job111161243743；48组344 pass/0 fail/7 ignored、CTest3/3、external17（含被拒文件guard）及固定真实模型/store/core/worker/HTTP/CLI/完整包/解压bridge通过；50项证据身份/hash已核；`native_window_tested=false` |
| 最新交付 | `Nexa-Windows-x64-33f0e17.zip`，11,119,286 bytes，SHA256`b37d89cbd1baf1dfd07d7dbdeae794157504d4c5a4064e171e7c4851d1015c1b`；原ZIP字节未改；817文件/runtime209、6个AMD64 PE导入闭包、许可595+6+198项及AWS-LC原文完整；3 CRT与CI签名记录一致，Linux未重新Authenticode验签；2026-10-03 13:42:49 UTC发送获接受 |
| 上一交付 | `Nexa-Windows-x64-43ad5c2.zip`，9,432,048 bytes，SHA256`5dc8cffe0fd4113b715a989566d481f5ff482099327d036e4768c2af7d66f7b5`；原ZIP字节未改；750文件/runtime197、6个AMD64 PE的普通及delay imports、许可540+6+186项独立核验通过；3 CRT与CI微软签名记录一致，Linux未重新Authenticode验签；2026-10-03 08:45:45 UTC发送获接受 |
| 较早交付 | `Nexa-Windows-x64-50c9d41.zip`，9,423,216 bytes，SHA256`713cd39d78adeb38e585529f3e188c9a3912090651172e3b268fb21bcab5c47f`；原ZIP内容未改；750文件/嵌套runtime197文件、6个AMD64 PE导入闭包及许可540+6+186项记录独立复核通过；3个CRT与CI微软签名记录一致，Linux未重新签名或验签；2026-10-03 07:03 UTC消息发送获接受 |
| 历史交付 | `Nexa-Windows-x64-389eeef.zip`，9,789,508 bytes；SHA256 `45251f28c2eb61a1b6ee5119aab3b0923a8117c677fef4ec91ea680be1b209f0`；750文件、嵌套runtime、6个PE与许可hash已独立复核；2026-10-02 13:36 UTC附件发送被接受 |

旧389eeef交付据[历史记录](docs/verification/2026-10-02-windows-model-compatibility.md#最终提交ci与交付)；50c9d41历史CI、包复核与交付范围见[开放模型记录](docs/verification/2026-10-03-windows-open-models.md#最终50c9d41-windows-ci与交付产物2026-10-03)。43ad5c2历史WindowsCI/包复核/交付见[混合目录记录](docs/verification/2026-10-03-mixed-model-directory.md#精确43ad5c2-windowsci与交付产物)。最新33f0e17的下载/CI/包复核/交付见[目录下载记录](docs/verification/2026-10-03-model-catalog-download.md#最终33f0e17-windows-ci与产物)。文档更新不表示用户已下载或运行。旧包 `bc43e0f3` 已有原生启动、导入、聊天、停止和两种关闭手验；不能追溯证明新目录版窗口操作通过。

## 当前路线状态

| 阶段 | 状态 | 下一步/边界 |
| --- | --- | --- |
| W00 主线收敛 | 已完成 | `82c4db6`独立审查、文档/归档检查与远端身份核验通过；纯文档无源码改动、无CI运行，见[本轮记录](docs/verification/2026-10-03-windows-scope-and-harness.md) |
| W01 当前版本短验 | 待验证 | 已发送33f0e17新包，待验Windows10自动发现/下载→显式扫描/加载、取消、混合目录诊断、零复制/自动名、剪贴板及独立API；CI bridge不替代窗口手验，等待用户目标机窗口 |
| W02 开放模型与CPU性能 | 进行中 | 用户要求16GB机器广泛模型支持；50c9d41完整WindowsCI及固定GGUF真实回归通过；独立包复核与发送完成；用户Win10/i5-8400/16GB验收与其他模型/性能仍待完成；本轮43ad5c2混合目录增量已过本机/独立审查及精确WindowsCI，独立产物复核及发送完成，用户目标机仍待验；本轮33f0e17自动发现/双源下载已过WindowsCI、MS固定模型实际传输与完整包链路，新包独立复核及发送完成；HF/其他候选/目标机仍待验；固定8条是建议目录非产品名单，见[开放模型记录](docs/verification/2026-10-03-windows-open-models.md) |
| W03 桌面管理器 | 未开始 | 托盘/窗口恢复与API诊断体验；当前已有服务启停和关窗保留服务 |
| W04 API / deepseek harness | 进行中 | 窄文本协议切片完成：pi-ai7场景、真实HTTP+合成执行器1项、8个native-free包248回归及clippy/独立审查通过；早期本地全workspace因缺子模块失败；35bfd85 WindowsCI324/0/7及旧模型真实链已通过，DSH/Windows pi-ai/生产工具未跑；新T0为13条无模型parser观察/CTest4/4及独立审查，定位缺口但不证明工具接受，见[分层记录](docs/verification/2026-10-03-windows-scope-and-harness.md) |
| W05 后期发行验收 | 未开始 | 无开发工具、实际离线、长期稳定性、升级/回退、Windows11及完整支持矩阵 |

各阶段最小增量、依赖与验收见 [路线](docs/roadmap.md)。目标机验收暂不可执行时，保留W02验证准备与未执行项；W04新实施按用户最新要求暂停，不降低验收门槛。

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
2. 收口aria2工作树与三补丁Windows源码构建，按同source运行真实源/进程/文件事务/完整包闭环；旧具体被拒分支仍未知，不预称修复。目标机下载→显式扫描/加载完整验收仍未完成
3. W02混合目录43ad5c2已通过WindowsCI、包内验收与独立下载包复核，原字节包已发送；等待用户目标机验收，按[本轮矩阵](docs/verification/2026-10-03-mixed-model-directory.md)逐层记录；旧50c9d41 CI不覆盖该增量。继续其他模型/目标16GB机实测，不扩大已验证矩阵。W04完整DSH/真实模型文本与工具能力缺口保留，按用户要求暂停新实施；pi-ai fixture仍非DSH本体捕获
4. ADR0017的33f0e17已过精确WindowsCI、真实MS固定模型链和独立包/新依赖许可复核并发送；后续记录用户目标机发现、下载→显式扫描/加载与取消分支。HF实际下载与其他7个候选另验。W04新实施暂停，生产工具仍未实现，不恢复Android或绑定Telegram业务

## 历史与保留工作

完整旧状态原文（含T00–T08、Android研究及历次失败/交付）见 [状态快照](docs/archive/windows-focus-2026-10-03/PROJECT_STATE.md)。既有验证报告仍原位保存。`apps/android-verifier/` 14项修改/未跟踪文件是暂停的B3b WIP，不属于本轮；不得删除、暂存或覆盖。它不构成可交付的新APK或生产支持。


### 开放模型提交与构建状态

8522514的Windows37101303658与历史MNN37101303651已cancelled，不记为失败推理或通过。闭包修正50c9d41的Windows37101760025已success，当前产物独立字节闭包复核已通过，原字节包于07:03 UTC发送获接受；目标机下载/运行仍待确认。50c9d41的历史MNN研究回归37101760095另已成功，只用于共享DTO迁移回归，不是Android App/设备验收，未恢复Android产品线；旧B3b WIP未动。详见[最终CI记录](docs/verification/2026-10-03-windows-open-models.md#最终50c9d41-windows-ci与交付产物2026-10-03)。
