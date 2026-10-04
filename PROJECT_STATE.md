# Nexa 当前状态

## 2026-10-04 本批关联功能整合（合并提交前验证快照）

用户指出平行功能分支容易冲突并已关闭PR #6，要求由开发方处理；随后明确要求长期维护一个开发分支，除非确需隔离不为每个功能开分支，覆盖此前逐功能从main开分支规则。已确认main `26206ef882e0d47d506767dd683aa32e20da111d`包含LAN c216722；添加模型 `206d965cb9a40c61b94b8fe8cb9c3ea3821eb7a2`来自更早main，导致公共文件冲突。当前将最新main合入现有 `codex/nexa-add-model`，保留两套功能并统一回归，不强推、不改main、不要求用户手工选边。

剩余Windows测试记录/模型按钮反馈、文件校验超时及不自动卸载，继续在本批同一开发分支按序完成，不再另拆给用户合并。已新建但无功能提交的 `codex/nexa-model-test-fix` 不再作为本批交付入口，未删除。合并冲突已解决：保留LAN与Add两边功能，新增7项交叉UI回归；联合全workspace40组437通过/0失败/7既有忽略，完整clippy/fmt、前端291项/typecheck/lint/build、Python153+2skip通过。独立bridge/store171、壳31与临时交叉验证通过；实际Windows窗口/两机LAN仍未验。详见[整合验证](docs/verification/2026-10-04-model-management-integration.md)。剩余修复完成后提供一个新的PR与一个联合Windows测试包；此前单片测试不转授合并结果。下文为各片历史快照，不覆盖本节最新工作流。

## 2026-10-04 可选局域网 API（提交前验证快照）

用户已手动合并模型使用流程到 main。已实际读取最新 main `6167d07cb523cc838e6a6fb082e660d56e9d7f79`（merge PR #4，tree `311936f4989a276c934d38d9da46a80fcc34bff7`），本次新分支 `codex/nexa-lan-api` 从该提交创建；后续每个新功能从当时最新 main 建独立分支。

- 新增方向见[ADR0020](docs/decisions/0020-opt-in-lan-inference-api.md)：默认关闭、独立LAN凭据/监听、具体私有IPv4与有限客户端CIDR名单，仅允许已本机加载模型的 models/chat。回环管理/proof不放宽，不自动修改防火墙、不提供公网/TLS服务
- Rust双监听/认证/调度与桌面设置实现并冻结。父全workspace/all-targets 40组421通过/0失败/7既有忽略，完整clippy/fmt、前端247项/typecheck/lint/build、Python155项（153通过/2平台skip）通过；独立审查22项为其中子集不累加，无剩余阻断。真实TCP只在loopback、私网peer为模拟，Windows网卡/两机LAN/原生剪贴板未验。详见[本轮记录](docs/verification/2026-10-04-lan-api.md)，联合功能包随后构建
- 用户另已授权“添加模型”按钮：选择单/多个GGUF仅校验并零复制登记所选文件，可选加载测试；属于后续独立main分支，不混入当前LAN提交。既有自动发现轻量跳过未变文件，但真正扫描仍全目录hash，此事实已向用户说明
- 用户另报告基础测试后仍全部无记录、按钮持续“尝试加载”：代码审查已定位Windows canonical VerbatimDisk数据目录被外部目录校验拒绝，scope错误又被吞成无记录；按钮还只依赖旧historical validated。独立修复排在模型添加之后，不混入LAN提交；另外已批准高级文件校验超时与“不自动卸载”设置，按顺序独立分支实现、最终统一回归
- MiniCPM5-2B-abliterated问题仅完成只读定位：同名公开候选头为llama架构/minicpm5分词器，固定vendor已有对应基础支持；用户具体来源/报错尚缺，不认定为架构不支持或模板已通过。本轮不改模型兼容/推理引擎

最近已交付完整Windows包为本地clean `1845f936`，18,937,607字节，SHA256 `4191ec7248a1413fed52d6ae03c43f31fd515a73271d47807ac42688f2428b40`；实际Windows新版运行待用户反馈。其52文件源码分批上传为远程 `9f836d5`，tree与本地包源完全相同，已由用户合并到上述main；分批上传成功，旧工具取消根因未确认。原包身份不改写为新remote提交，也无需重下载。未运行GitHub Actions Rust。

## 2026-10-04 选中文件添加模型（提交前验证快照）

用户明确选择单/多文件添加、取消默认自动全库扫描，并要求依次完成 LAN、添加模型、基础测试记录/按钮反馈、高级文件校验超时及不自动卸载。LAN源码已在独立 `codex/nexa-lan-api` 提交 `c216722fd6913208b0529cf3db247729ab7f4019`，源测试421/0/7、前端247、Python153+2skip通过，WindowsLAN尚未验；本分支不夹带该源码。

当前 `codex/nexa-add-model` 从最新 main `6167d07cb523cc838e6a6fb082e660d56e9d7f79` 新建。按[ADR0021](docs/decisions/0021-selected-file-model-registration.md)实现schema2跨目录显式文件来源、原生单/多文件选择、仅选中payload校验、默认只读浏览、下载定向登记和手动全量维护；源码实现冻结；全workspace/all-targets 40组423通过/0失败/7既有忽略，完整clippy/fmt、前端230项/typecheck/lint/build、Python155项（153通过/2平台跳过）通过；独立两crate169、壳31与UI118均为各自回归子集不累加。最终Windows壳及两crate交叉检查通过，原生选择器/写删锁与真实GGUF未在目标Windows执行。详见[本轮验证](docs/verification/2026-10-04-selected-model-registration.md)，联合包待其余切片完成后构建。

已知独立后续修复：Windows `ModelStore::open` 的 canonical VerbatimDisk 数据目录被旧外部目录语法拒绝，使基础测试scope/记录失败后被界面隐藏为未测；按钮也只依赖历史validated。这里只读确认，尚未修改或Windows实机复现。MiniCPM具体模型仍缺用户来源/错误码，不将文件名当作不支持结论。

各片完成后统一回归、构建新Windows测试包；不在GitHub Actions编译Rust，不修改用户防火墙。最新已交付仍1845f93包（源码已由9f836d5并入main），Windows新流程待用户反馈；下文历史快照不覆盖本节最新顺序。

## 2026-10-04 模型自动登记与本机基础测试（提交前验证快照）

本节为最新状态，覆盖下文历史“当前/待交付”安排。用户确认已交付 `688fe5c` 完整交叉测试包可以下载模型；未给出具体模型/源/hash，不能扩展为全部下载源或 Windows 完整验收通过。此前包为 18,674,810 字节，SHA256 `f7edd5c8ab204d0a326905aca5d98d0cb97eef5706350d94a6e65f6ffea1bd6e`。

- 本次按 [ADR0019](docs/decisions/0019-model-onboarding-and-local-validation.md)接通下载后自动登记、可选空闲自动加载/基础短测、本机验证记录与停止服务时的只读模型列表
- 启动/进入模型页/刷新有界发现外部新增完整 GGUF；服务运行中只显示待登记，不暗中停服或替换已加载模型。被动浏览不启动服务；下载页显式选项默认开启，旧调用未传选项默认关闭
- 本机“加载成功/基础生成通过”与历史验证矩阵分开；绑定真实模型/模板/引擎/参数和平台，条件变化则失效。落盘失败、空输出、断流、取消或延期不能用旧 Passed 冒充本次通过
- 最终源代码回归：Rust 全 workspace/all-targets 40 组 407 通过/0 失败/7 既有忽略，完整 clippy/fmt 通过；前端193项/typecheck/lint/production build通过；Python155项中153通过/2平台跳过；独立源码与断连/竞态复核无剩余阻断
- Linux 真实 Qwen3-0.6B Q8_0 已完成加载、短生成、记录持久化、停服后离线读取与生命周期清理；报告 success/local_text_validation/offline_inventory 均 true。该证据不冒称 Windows 外部文件保护或窗口通过，见[本轮验证](docs/verification/2026-10-04-model-onboarding.md)
- 下一步：形成源码提交并捕获干净源身份，执行新 Windows 交叉构建、真实同源 aria2 重建与完整 ZIP 核验后提供测试包。当前新包尚未完成；不运行 GitHub Actions、不购买资源、不改 vendor/推理 ABI、不恢复 Android/Harness

## 2026-10-04 云端 Windows 交叉构建（提交前验证快照）

用户因本地构建反复失败，已要求改由云端构建，并明确同意本次 Microsoft Build Tools/SDK 适用条款。当前使用 Linux 云端的独立 Windows x64 MSVC-ABI 测试路径，不使用 GitHub Actions、不购买云资源；用户后续负责运行验收，无需继续自行编译。原生 Windows 两个打包器保持原样，见[交叉测试构建说明](docs/windows-cross-test-build.md)。

- 基线 `ecfa2c21aee52b65957dde9de534ca75704ca7f9`，真实锁定 llama.cpp Git checkout 为 `2149c00f4442dc59302e134a02e4c99d5f7ed9fc`；主仓库使用核对原始 Git object SHA 的显式 shallow sparse checkout，未物化 Android/历史文件不冒称完整 checkout
- 新增显式 `linux-clang-cl-msvc` profile，核对 Clang/MSVC frontend/ABI、Linux host、Windows x64 target、Release /MD、实际编译探针及与 i5-8400 对应的固定 AVX2 等 CPU 基线；不修改 vendor、不伪装 MSVC 编译器身份
- 实际探索构建已完成 Windows AMD64 的 desktop、API CLI、worker、验收器以及锁定推理静态库；前端149项、typecheck/lint/production build通过，新增身份/打包器测试及独立审查通过。探索性构建不充作最终 commit 的交付证明
- 新独立 cross-test 打包器保留产品既有三层来源身份、PE普通/延迟导入闭包、原许可与同源aria2；真实Linux签名校验与Windows系统验签/运行分开记录。工具链目录声明hash/大小异常保留在完整来源报告，不能写成所有上游目录链已通过
- 最终提交后必须重新捕获源身份、执行最终构建并重建相同提交的aria2，再核验完整ZIP与生产身份消费者。当前仍待最终包；Windows窗口、真实模型、下载和目标Win10运行均未执行，不能宣称已验收或公开Release
- 本轮不迁移pnpm、不修改Rust选择方式；后续构建工具最低版本和前端迁移继续单独处理

下文为此前本地构建阶段快照；本节覆盖其“必须用户本地编译”的安排。

## 2026-10-04 CMake 最低版本修复（提交前验证快照）

用户已明确本次先修复 CMake 并提交：本地 Windows 打包最低版本统一为 **CMake 4.2.0**，不再要求精确 4.4.3，4.4.4 及后续版本通过数值门槛；同时检查安装的 CMake 提供所选 VS 生成器。实际版本继续记录到 manifest，不把放行等同完整构建通过。本次不改变 Rust 工具链选择或前端包管理器，Rust 与 pnpm 后续单独处理。脚本检查和边界见[本轮验证](docs/verification/2026-10-04-windows-vs-selection.md#cmake-最低版本后续修复)。

本节覆盖下文“CMake 4.4.3 锁不变”的旧要求。此前 VS 修复已提交 `241146e57686161c3bda059d8f2f36bd3754eac1`，对应 aria2 构建输入已另行提供；新提交仍需匹配其来源身份的组件，不能混用旧包。用户 Windows 实际构建和新下载引擎运行仍待验证，不运行 GitHub Actions Rust 构建。

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
