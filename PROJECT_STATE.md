# Nexa 当前状态

最后更新：2026-10-02。状态区分工程实现、Linux 开发验证、Windows CI 与目标设备验收，不把任何一项互相替代。

## 当前目标与授权

用户已明确要求按照文档规划实施，并允许本地检查通过后推送新的开发分支、增加和运行 Windows GitHub Actions。未授权合并或部署。首个业务仍是 Telegram 群摘要；来源、触发、样本与保留策略保持待决，不自行登录账号或向群发送摘要。

Windows 10为首要交付目标，Windows 11后续增加；Windows保留llama/GGUF。Android已确定采用MNN，CPU、OpenCL、QNN v79/v81与直接Hexagon均纳入计划，不先做Android llama。面向Snapdragon 8 Elite及后续，公开首测参考OnePlus 15 / SM8850 / v81，SM8750 / v79为兼容档；实际系统、驱动、ABI/页大小、内存和持续性能待诊断。方向与门槛见[ADR0008](docs/decisions/0008-android-mnn-engine-and-package.md)及[执行计划](docs/t07-android-mnn-plan.md)，不从型号推导支持或性能保证。

无开发工具、离线运行和长期稳定性列为后期验证。T05以已完成的Release CI、完整包检查及独立Windows 10手工短验按当前阶段范围收口；A20/长期稳定性仍未验证，不阻塞T06当前开发。

## 已有工程事实

- 原始远程提交 `0d3a3cea32b813dad0857f9e1a1e41862ce27168` 已通过 GitHub 原始对象精确重建本地 Git；初始 tree/commit SHA 一致，未创建替代历史
- Rust workspace 已有runtime-types、model-store、runtime-core、engine-host、llama-adapter、xtask；T03已有runtime-ipc、process-host、runtime-worker，Linux聚合及固定Windows进程隔离/真实模型已通过；T04新增HTTP/CLI并已完成固定Windows CI阶段验收；T06现已有desktop-bridge和React/Tauri桌面工程，Windows原生构建/桌面诊断/完整Release bridge已通过，旧包原生启动/导入/聊天/停止生成及两种关闭已有独立手工验收确认，剪贴板等其余分支待验证；仍无移动工程
- llama.cpp submodule 锁定 `2149c00f4442dc59302e134a02e4c99d5f7ed9fc`；Rust 1.98.1 与 Cargo.lock 已锁定
- 自有 C ABI 实现模板、分词、精确逻辑预算、prefill/decode、采样、跨 token UTF-8/stop、取消和资源释放；Rust 使用借用/线程约束及 panic 隔离
- Qwen3-0.6B Q8_0 实际文件和模板 hash 已核对；来源、许可与固定参数见 [模型矩阵](docs/model-matrix.md)
- Linux CPU 与 Windows x64 CPU CI 已完成真实中英文流式、重复加载、stop/预算/取消/恢复和 A02 system/多轮模板与请求间隔离
- 提交 `d3d7cf2d9f0d2ce7aa03ca7d27787a2f423f3144` 的 [Windows CI](https://github.com/Naza3/Nexa/actions/runs/36791679663) 全部通过：固定模型/模板身份、上游生成及五次 bench、自有九场景/十轮 suite、两项真实恢复/多轮测试
- 本次支持证据限 Windows Server 2022 x64 / EPYC CI / 2 逻辑 CPU、2 推理线程、固定 Qwen3-0.6B Q8_0 / context 2048。1 线程短探针通过，4 线程超配短探针仍在 60 秒超时，不能泛化为任意线程配置；原始失败完整保留于报告

构建参数见 [构建锁](docs/build-lock.md)；当前可执行命令见 [xtask](xtask/README.md)；验证见 [本轮记录](docs/verification/2026-09-30-native-baseline.md)。

- T02 已实现受控模型导入/manifest、单一调度actor、有界FIFO/输出、三类deadline、空闲卸载与专用原生线程；Linux与固定Windows配置的真实store→core→host→shim链路均通过；实现提交`bc316da6a66eb52a24ee7a5cb56d8f8c45d1ad37`的[Windows CI](https://github.com/Naza3/Nexa/actions/runs/36796147278)于2026-10-01 00:37 UTC完成
- shim build_info升2，保留旧air_generate并新增数值进度观察；Linux与Windows均在成功prefill批次后及decode阶段分别跨线程取消；Windows单次mid-prefill为3.4101ms、经actor取消至终态15.7851ms，不作普遍延迟保证。实测见 [T02记录](docs/verification/2026-10-01-t02-runtime.md)

- T03实现提交`a8930494f62909cccb11876011a650d9713bc74c`的[Windows CI](https://github.com/Naza3/Nexa/actions/runs/36801681068)于2026-10-01 01:47:46 UTC全部通过；ZIP摘要与精确提交/模型身份已复核。独立无native管理程序真实生成、取消与回收成功；极端清理未确认永久fail-closed，不虚报reaped

- T04实现提交`ccb2053fe514f582f6161f9fc87ee25346aa55e4`的[Windows CI](https://github.com/Naza3/Nexa/actions/runs/36816604494)于2026-10-01 05:00:03 UTC全部通过；212项Rust测试（平台差异已核对）、5项独立真实模型、DACL/Job实际边界及HTTP 50次断连恢复均通过。ZIP摘要已核对，完整证据见[T04记录](docs/verification/2026-10-01-t04-http-cli.md)

## 任务状态

| 任务 | 状态 | 当前边界 |
| --- | --- | --- |
| D00 文档与总体设计 | 已完成 | 原文档基线保持；当前工程事实已同步 |
| T00 工程与基线 | 已完成 | 固定组合已在 Windows CPU 完成真实上游生成和五次 bench；输入、统计与构建证据已归档 |
| T01 原生链路 | 已完成 | 最小阶段门槛通过 Windows 真实中英文流式、重复加载释放、模板/特殊 token、取消和恢复；A08独立prefill已补Linux/Windows观测 |
| T02 调度与存储 | 已完成 | 代码bc316da6及固定Windows 2线程/context2048组合通过；A05–A12按逻辑测试/真实链路分别留证，不代表HTTP或进程隔离 |
| T03 PC worker | 已完成 | a8930494在Windows CI36801681068通过：Job父子/后代回收、五秒强杀、单账本IPC、144项Rust+5项真实模型及独立无native父端链；范围见T03记录 |
| T04 HTTP/CLI | 已完成 | ccb2053在Windows CI36816604494通过：212项Rust、37项API分层测试、真实DACL/Job、5项旧真实回归、无native CLI及真实HTTP50（89pass/9skip）；Linux214项回归通过；范围见T04记录 |
| T05 Windows发行 | 已完成 | 6a7e9d0的CI36829233039已通过真实Release便携包/独立工具、PE与许可/hash、中文空格路径和HTTP50；独立Windows 10短验已通过；按当前阶段范围收口；A20无开发工具/离线及长期稳定性移至后期验证 |
| T06 Windows UI | 待验证 | 新目录/诊断源码75e458f的CI36948947690已成功，native job110657335010含真实模型/runtime/HTTP/CLI、桌面包及解压bridge验收通过；产物独立复核通过并已交付；新目录原生UI仍未测 |
| T07–T08 Android核心与UI | 未开始 | MNN方向/六切片文档已就绪，尚无MNN适配、工具链锁、模型包、APK或真机证据；既有llama build.rs仍拒绝Android |
| T09 发布验收 | 未开始 | A01–A26完整矩阵未执行 |
| T10 平台/后端扩展 | 未开始 | Android后端纳入T07-D～F单列验收；其余扩展未开始，Linux仍仅开发探针 |
| S00–S04 摘要 | 未开始 | 来源/触发/样本/质量目标及接口契约仍待冻结 |

## 重要实现与验证界限

- 模型的原生 context 分配会向上按256取整；Nexa 单独保存用户请求的逻辑context预算，真实33-token边界回归已覆盖，不能借分配扩容放宽预算
- 上游 `llama-completion --reasoning off` 未在初始prompt分支传关闭参数；基线使用锁定原模板渲染的固定合成prompt。自有shim直接关闭思考，不剥离输出标签
- 回调仍为同步借用，现允许共享预算内可取消等待；T02实现256KiB预算、4KiB UTF-8分片、10秒无消费进展时限及协作式deadline；T03已在固定Windows CI验证五秒kill、Job父子/后代回收、跨进程单一信用及消费lease；Linux不承诺异常父退出后任意孙进程回收
- Linux与Windows已补独立prefill中途取消及decode取消观测；100短请求/20加载的长期内存趋势及目标硬件表现未完成；Linux数据仅代表共享开发机
- ASan/UBSan纯流缓冲测试通过；LeakSanitizer因沙箱ptrace不可用，未宣称原生库通过完整内存泄漏检测
- 摘要文本质量、证据归因、Android后台生命周期、正式Windows发行能力均未验证

## 下一步

1. T00–T04阶段证据已收口，继续授权开发分支，不合并或部署
2. T05源码6a7e9d0的Windows CI36829233039已通过，产品/工具ZIP已独立复核；另有Windows 10独立手工短验通过，不能替代尚未完成的A20条件
3. 新源码75e458f的[Windows CI36948947690](https://github.com/Naza3/Nexa/actions/runs/36948947690)已成功；下载产物已独立核验并交付，证据见[T06最终目录版记录](docs/verification/2026-10-01-t06-desktop.md#第七轮目录版windows-ci成功独立复核与交付2026-10-02)。新目录原生UI仍未测，剪贴板等原生分支继续待验
4. 无开发工具、实际离线和长期稳定性列为后期验证；Windows11/Android另行实测，不泛化当前支持证据
5. Android从T07-A精确MNN/工具链与CPU原型开始；新增SDK/许可和实际设备条件分别核验，未获批准不下载SDK或接受协议。T07-B再冻结具体C ABI/schema/公共类型迁移
6. 摘要来源/触发/评估基线保持独立待决；基础runtime推进不自行选择Telegram产品方案

## T05 收口与后期验证

[ADR0006](docs/decisions/0006-t05-windows-portable-package.md)冻结产品与验收器分离、既有VS Release CRT app-local依赖闭包、严格文件/许可/hash以及失败保留；[T05验证](docs/verification/2026-10-01-t05-windows-package.md)分层记录真实结果。只在开发Actions上传私有验收产物，不创建公开Release、不合并部署、不远控或初始化长期真实token。项目root LICENSE未选是后续外部分发决策，不阻塞本轮内部开发包。

最新实际结果：2026-10-01 07:39:43 UTC，[Windows CI36829233039](https://github.com/Naza3/Nexa/actions/runs/36829233039)全部必需步骤成功。产品ZIP4,936,580bytes，工具ZIP1,516,822bytes；解压Release包16项检查通过，HTTP89pass/9skip、50次断流恢复、四类中文空格路径和退出清理均实测。前四轮失败及修复保留在[T05记录](docs/verification/2026-10-01-t05-windows-package.md)。另有独立Windows 10手工短验通过；该结论不覆盖无开发工具、实际离线或长期稳定性，A20仍未完成。


## T06 当前实现与待验边界

[契约](docs/t06-desktop-contract.md)、[ADR0007](docs/decisions/0007-t06-desktop-shell-boundary.md)、[T06验证](docs/verification/2026-10-01-t06-desktop.md)记录当前范围。`apps/desktop/`前端独立npm锁，`src-tauri/`独立Cargo锁，根新增`desktop-bridge`；原15命令现扩展为20个固定IPC命令、main本地ACL/CSP、一次性原生选模与原生写剪贴板、已安装WebView2检测、异步统一关闭均已有源码。

已实际完成：前端锁重装、typecheck/lint/build、36测试和npm audit 0；Linux真实模型bridge两种独立进程退出语义通过。Windows已多次完成真正Tauri Release、源码clean、PE/原许可闭包、完整T05包16项及HTTP89pass/9skip、50/50/50断流；实际桌面诊断包验证通过，已装WebView2为131.0.2903.86。早期同EXE对照观察宿主Job下BREAKAWAY返回OS5、继承Job则正常启动/退出且Ctrl+C pending；生产仅移除BREAKAWAY，保留DETACHED/NEWGROUP，不改Job/权限或CLI信号。

先前交付源码`bc43e0f3ac215d41e5d93cccf670ab43d67d41c0`、tree`beb8647c1e6494591f36af38535554f7678719d0`的[CI36864041027](https://github.com/Naza3/Nexa/actions/runs/36864041027)于13:31:29 UTC已确认completed/success：Windows根278pass/0fail/6ignored、壳4tests、早期Rust传输8tests、Python66tests全部通过；完整桌面Release bridge真实导入/生成/取消再生成、同实例连接、实际宿主进程退出后API仍可访问、同时退出回收worker/实例及空闲设置语义全部通过，五类实际路径均含中文和空格，包未被修改。

桌面原ZIP9,507,755bytes，SHA256 `2ea95591ddabc4e7ae930ea166e6343f6aad275fa01975d3eb7bebbef0546fe9`；私有Actions artifact`11166511363`。750文件、嵌套runtime同source/tree、所有hash/许可、6个PE/import闭包均已独立重算核验。安装29,501,813bytes，其中完整runtime14,203,784bytes、UI EXE10,573,312bytes，模型0bytes。工具等待缺口已通过真实Windows早期回归和完整Release链验证；此前失败/取消证据均保留于T06记录。CI原生窗口报告仍为`native_window_tested=false`；独立手工结果与CI分层记录，T06仍有原生UI分支及新目录功能待验，不冒充完成。

旧包bc43e0f3已有独立手工验收确认原生启动、导入、聊天、停止生成、默认关闭保留服务和同时退出六项行为；剪贴板与其余错误恢复仍待验证。另在受控解压产品包中加入[模型矩阵](docs/model-matrix.md)锁定的公开Qwen3-0.6B Q8_0输入，已独立复现旧版严格文件集合误拒同级GGUF；来源是本地工程复现，不能记为新版本Windows运行通过。

当前源码已将程序根及固定model/models目录的直接普通GGUF识别为用户输入（只读四字节头、不自动导入、不计payload），保持声明文件/hash/source、runtime/licenses、未知DLL/EXE/嵌套目录及reparse拒绝，并保留固定启动错误码。Linux原包+真实GGUF已复现修复前失败/修复后通过；新行为及实际EXE三处模型正例、DLL/manifest篡改负例纳入75e458f已成功的Windows CI；下载产物独立复核已通过，不能追溯为旧包已支持。

云浏览器127.0.0.1预览曾被客户端阻止，未完成浏览器视觉交互；Windows 10剪贴板与其余错误恢复等仍待独立手工验收。Nexa不创建kill-on-UI-close的runtime Job，尊重外部宿主Job/会话整体终止，不承诺脱离其生命周期。T05阶段范围及后期无开发工具、离线、长期稳定性验证边界不变。

当前工程要求：设置可选择任意支持的本地模型目录、已有GGUF直接读取不再复制，并按文件名自动命名。按[外部目录契约](docs/t06-model-directory-contract.md)推进：native folder picker/一次性选择，源目录只读、非递归；私有索引和token仍在AppData，不迁移删除旧managed模型。目录应用/重扫须先停止服务，不自动shutdown；有效目录必须由同TCP proof后的服务身份确认。当前壳/核心/UI和同级GGUF诊断修复已接通并完成本地聚合检查；源码`75e458f60cbbfc2b136d8396d7e824c3fc07f23e`的[Windows CI36948947690](https://github.com/Naza3/Nexa/actions/runs/36948947690)已成功，native job`110657335010`完成真实模型/runtime/HTTP/CLI、桌面打包及解压后bridge验收。此前Linux真实旧managed链及external通用观察与MSVC check分层保留；下载产物已独立完整性复核通过并交付：原ZIP9,785,013bytes，SHA256 `1d4f89eeb9c03b14aecaa7199c847413ee85b215cd44ee8bf9436bbb14596858`；source tree `8cec0b3be1d8c4f3442d72f2c27f87f8506d9523`。壳23tests、Python83tests（2 Windows-only skip）、目录版前端60tests及对应typecheck/lint/build已通过；根workspace最终独立复核306pass/0fail/6ignored及strict clippy/fmt通过。本轮CI与产物独立核验通过不等于原生窗口已操作；新目录选择/零复制/自动名称的原生UI仍未测，不将旧包手工验收追溯为新功能通过。

## Android MNN 文档迁移

[ADR0008](docs/decisions/0008-android-mnn-engine-and-package.md)已记录方向并同步架构/规格/路线；[计划](docs/t07-android-mnn-plan.md)拆为CPU原型、MnnExecutor/包/安全契约、前台APK、OpenCL、QNN v79/v81、直接Hexagon六切片。当前只是文档完成；MNN3.6.1完整commit为研究候选，未进入实际构建锁，所有Android实现和设备门槛均未开始。新schema/ABI/profile字段未冻结，不新增外部协议承诺。Windows源码与当前CI状态独立，不因本迁移改变。
