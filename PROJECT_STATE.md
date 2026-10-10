# Nexa 当前状态

## 2026-10-10 Rust 错误规范（本地主机验证完成）

任务 W05-RUST-ERROR-1：在 codex/dev 9b18f41 基线上新增[全项目规范](docs/rust-error-handling.md)，类型化启动与私有文件错误，取消按 Display 字符串分类，保留安全公开诊断与底层 source；统一两个 workspace 的错误结果/调试宏 lint。本轮本地中文提交，未推送。

用户批准后已在隔离云端目录恢复官方Rust1.98.1。根native-free范围631测试通过/4既有忽略、Linux桌面壳33测试通过、两范围strict Clippy/fmt、Python367通过/5平台skip和diff空白检查通过。首次Python因缺子模块许可文件失败，恢复锁定源码后重跑通过。两Cargo.lock不变。Windows与原生推理三个crate本轮未验证，不声称全平台或全项目所有历史错误均完成迁移。详见[验证记录](docs/verification/2026-10-10-rust-error-conventions.md)。

此前9b18f41的原生验证已成功并交付开发测试包（[Actions37931429259](https://github.com/Naza3/Nexa/actions/runs/37931429259)）；下方“原生安装验收启动竞态修复（验证中）”是旧提交内的历史快照，不是仍待排障。本轮改动不能继承该批二进制验证。版本仍0.3.0，main和正式Release未变。

## 2026-10-09 原生安装验收启动竞态修复（验证中）

错误处理与模型显示名已获授权提交并推送至`codex/dev`：`085cf4024ec5f742e8b46e91210dd3511de01f8c`。原生CI [37926350189](https://github.com/Naza3/Nexa/actions/runs/37926350189) 的构建/单测通过，但安装生命周期在NSIS重复安装后的runtime启动阶段失败；安装包仍为UNVERIFIED，不交付为已验证包。Windows根Rust655通过/10忽略、桌面34通过、Python366项（364通过/2skip）、CTest5项已独立核对证据。

受控宿主复现确认验收脚本立即调用status存在抢占serve实例锁的窗口，但原失败未保留原因，不能断言该次必由锁竞态造成。修复验收为全新数据目录、无旧discovery，等待新进程发布discovery后才执行原认证status；保持30秒期限、存活检查、全部安装门槛与原停止确认。增加固定`runtime_instance_busy`启动码及有界白名单诊断/退出码，产品锁不绕过、serve不重试。修复后正在重新执行本地门槛和原生CI，结果见[验收记录](docs/verification/2026-10-09-installer-startup-order.md)。

以下两节为推送前验证快照；当前发布状态以上节为准。

## 2026-10-09 模型发现显示名（与错误处理联合门槛通过）

任务W05-MODEL-NAME-1。按用户确认，仅为本机/LAN `/v1/models`兼容增加已有`display_name`，稳定ID、分页、模型调用/历史与LAN仅驻留范围保持；缺少名称元数据时回退ID，不改索引或客户端。

官方PI Desktop v0.17.0实际源码解析/设置搜索/名称JSX隔离测试通过；发现列表可读取显示名，但聊天选择器仍按用户别名或完整ID显示，需要在客户端“高级→别名”设置，不能声称本修复自动改变聊天标签。当前安装版本/原生GUI未验证。

最终组合源码重新通过根Rust653项/10既有忽略、Linux桌面壳33、前端49文件1006、严格Python366项（361通过/5平台skip）与native身份5项；两host strict Clippy/fmt/typecheck/lint/build均退出0。两workspace Windows Release strict Clippy、10份native静态库、四EXE实际交叉链接与AMD64 PE/import复核全部通过，442个非Markdown代码文件前后hash一致。保留既有clang-cl探测、SDK缺PDB调试信息及前端大chunk warning，未关闭检查。

结果仍为基线2a56b396上的未提交dirty工作树；未推送、未tag/Release、未生成新安装包，版本0.3.0保持。完整Windows原生CI/安装器生命周期、目标Win10、真实模型与PI Desktop GUI待后续独立验证。前轮三份旧验证文档保持，不把旧CI授予本轮代码。见[ADR0043](docs/decisions/0043-model-discovery-display-name.md)、[名称说明](docs/model-display-names.md)、[本轮联合记录](docs/verification/2026-10-09-model-display-names.md)。

## 2026-10-09 全项目错误处理与LAN启动恢复（本地联合及Windows交叉验证通过）

任务W05-ERROR-1 / W05-LAN-RECOVERY-1。实现覆盖启动诊断、配置固定原因、模型/推理错误展示、下载清理及不确定结果、安装预检说明；不声称穷尽所有错误。可选LAN仅OS绑定失败保留认证本机管理，明确未运行原因，原配置/密钥/安全边界不变；修正须显式停服。启动私有stdout只传固定码，默认stderr仍null；前端统一有界脱敏，活动不把未确认读回写成完成，业务请求不自动重放。

冻结工作树通过根Rust652项/10既有忽略、Linux桌面壳33、前端49文件1006、严格Python366项（361通过/5平台skip）、native身份5项；fmt、两host strict Clippy、前端typecheck/lint/build全部退出0。联合门槛发现并最小修复xtask测试漏reason字段和安装Python夹具漏encoding，再完整重跑通过。

复用已核验缓存完成两workspace Windows Release strict Clippy、10份native静态库身份及Runtime/worker/acceptance/desktop四EXE实际交叉链接和AMD64 PE/import复核；442份非Markdown源码构建前后hash一致。另复核三安装helper实际交叉链接及PE，未生成本轮发行包。保留clang-cl探测、SDK缺PDB调试信息与前端大chunk warning；没有关闭检查。

本轮仍为基线2a56b396上的未提交dirty工作树，未推送、未tag/Release、版本仍0.3.0；不把旧CI赋予新代码。目标Windows GUI、真实局域网、新安装器原生生命周期、关闭桌面保留服务后的真实调用仍待独立验收。模型显示名是后续独立事项，未混入本次。见[错误恢复契约](docs/error-handling-and-recovery.md)、[ADR0042](docs/decisions/0042-lan-bind-startup-recovery.md)与[联合验证](docs/verification/2026-10-09-error-handling.md)。

## 2026-10-09 移除启动未声明文件检验（原生验证与产物复核通过）

任务W05-LAYOUT-1。按用户要求，启动layout不再扫描或拒绝未声明文件/目录，NSIS生成的uninstall.exe不再触发该拦截。声明文件hash、来源/校验表、必需EXE清单成员及路径防护保持，模型准入、ACL/token与数据配置未改。

用户批准推送后，精确源码 `2a56b39602806fbaf3c74841a5a8b0d1534bcf77`（tree `3a3db727e5b78b1bf353319f06c72e3dd6d9e2a7`）先通过干净提交的完整本地Windows交叉门槛，再正常更新codex/dev。[Actions37910225699](https://github.com/Naza3/Nexa/actions/runs/37910225699)于2026-10-09 09:55:05 UTC成功，五个构建/验证job通过，分支release job正确跳过。

Windows根Rust627通过/10忽略、桌面34、前端970、Python364项（362通过/2skip）、CTest5/5，真实模型/HTTP/CLI/bridge与17项安装生命周期通过。已安装MSI/NSIS的两个EXE诊断均package_verified=true、dirty=false、精确source一致；真实额外DLL正例和manifest篡改负例也通过。独立复核完整六文件发行库存、源码/hash/大小/安装证明及三份证据归档的62个文件通过，69事件安装诊断status=pass。

[本次三格式测试包](https://github.com/Naza3/Nexa/actions/runs/37910225699/artifacts/11608851406)有效至2026-10-16 09:54:57 UTC。版本仍0.3.0，未创建tag/Release或合并main，旧正式Release保持。本次交付未签名；Windows Server 2022的安装包诊断不能代替用户Win10原生GUI/配置验收，MSI独立configuration_unavailable仍未由此证明解决。随后仅补记证据的文档提交不冒充此批二进制来源。详见[ADR0041](docs/decisions/0041-declared-payload-startup-validation.md)及[完整验证](docs/verification/2026-10-09-declared-payload-validation.md)。

## 2026-10-08 版本0.3.0、推送与tag（已完成；标签发行构建进行中）

用户明确授权统一版本至0.3.0、推送GitHub并创建v0.3.0。已fetch确认main 1c3650c为当前开发分支祖先；远端codex/dev a18e9a1为本地祖先，v0.3.0尚不存在。一键工具同步七版本文件，重复执行零修改；两Cargo锁及npm锁第三方依赖不变，锁定离线metadata和严格Python检查通过。

本轮包含前轮Tauri MSI/NSIS、托盘和开机启动。修复Windows测试短路径比较和Tauri私有安装夹具跨盘资源读取后，干净a7db515先通过完整本地Windows交叉门槛，再推送codex/dev。[Actions37758119918](https://github.com/Naza3/Nexa/actions/runs/37758119918)五个构建/验证job全部成功，17项实际安装生命周期门槛全部通过；下载六个发行文件并复验清单、哈希与来源一致。

附注v0.3.0已创建并推送，远端确认指向a7db515f649f8ba05f3837c050b6ed56b920968c；main未自动合并。[标签发行Actions37762270527](https://github.com/Naza3/Nexa/actions/runs/37762270527)已启动，正式Release公开和附件上传仍以该流水线最终结果为准。源码/版本/tag交付完成，用户Windows10/i5的原生窗口与登录启动实际体验仍独立待验。命令、结果及限制见[0.3.0验证记录](docs/verification/2026-10-08-release-0.3.0.md)。

## 2026-10-08 Tauri 安装器、托盘与开机启动（本地检查与Windows交叉构建完成）

按用户选择，将自研 Setup/WiX 打包迁移为锁定 Tauri CLI 2.12.1 的 WiX MSI + NSIS EXE，保持每用户固定目录和完整数据保留，支持同格式同版本替换。旧 Setup 内含 MSI，应使用新 MSI 原位更新；跨格式直接覆盖拒绝，切换需先卸载程序。NSIS 替换不承诺事务回滚。生产 CI 接入新的17项原生生命周期门槛；本轮未推送、未创建tag，版本仍0.2.3。

新增独立、默认关闭的“关闭到托盘”和“开机启动”。关闭到托盘保存在既有 TOML，仅隐藏并保留任务/服务/模型；托盘显示/退出复用安全保存与关停。开机启动以当前用户 Windows Run 登记为真实持久化来源，带引号无参数、260字符限制、写后回读；正常卸载只清理属于本安装目录的项，升级保留选择。

前端970项、Linux壳42项、workbench11项、严格Python359项（354通过/5跳过）、fmt/typecheck/lint/build/actionlint通过。最新原生安装helper实际交叉链接和导入检查通过；完整Tauri→NSIS探测使用旧0ecddad载荷，解包28文件逐一一致，未冒充新功能发行包。干净源码f0a7183通过完整Windows Release交叉门槛，两workspace strict Clippy、四EXE实际链接和源码/PE核对全通过，证据20261008T081623Z。20份原始许可模拟Windows换行转换后字节一致；真实Windows安装/托盘/登录启动仍待验。见[ADR0039](docs/decisions/0039-tauri-windows-installers.md)、[ADR0040](docs/decisions/0040-desktop-tray-and-autostart.md)、[操作说明](docs/desktop-startup.md)和[本轮验证](docs/verification/2026-10-08-tauri-installers-and-startup.md)。

## 2026-10-08 PI Desktop OCR 插件下载发布（已完成）

用户明确同意公开发布后，插件 `pi-ocr-v0.1.0` 已发布为独立预发布，标签固定源码 `93cdf6d`，Nexa主程序仍0.2.3。[直接下载](https://github.com/Naza3/Nexa/releases/download/pi-ocr-v0.1.0/io.github.naza3.nexa-ocr-0.1.0.piplug) / [发布页](https://github.com/Naza3/Nexa/releases/tag/pi-ocr-v0.1.0)。安装包334150字节，实际HTTP下载200、SHA及每字节与已验本地产物一致；Windows宿主实机条件不因发布而改变。

云端可写Release API但uploads域官方CLI返回401，改为独立上传工作流。先在干净 `0f0e0fb` 通过本地Windows全交叉门槛，再仅推送专用 `codex/pi-ocr-publish`；[Actions37729949673](https://github.com/Naza3/Nexa/actions/runs/37729949673)两job成功，重现完全相同包并使用GitHub自身令牌上传，随后公开发布。未更新main/codex/dev远端、未触发主程序Windows发行，未覆盖旧Release。详见[发布验证](docs/verification/2026-10-08-pi-desktop-ocr-plugin.md#github公开下载交付)。

## 2026-10-08 PI Desktop 的 Nexa OCR 插件（安装包与Linux验证完成，Windows待验）

任务W04-PI-OCR-1。按用户“先完成插件”新增独立0.1.0 `.piplug`，位于 `integrations/pi-desktop-ocr`：本机服务身份验证、显式令牌导入、空闲自动加载所选已登记视觉模型、顺序20图队列、Markdown/原文、正文下真实性能、最近100条非空结果和TOML参数保存。面板关闭继续后台，禁用/退出尽力取消保存；Nexa核心与公共协议、版本0.2.3保持。

52项单测、官方devkit打包/检查、11项官方child契约、真实Controller/Store的Chromium联调均通过。生产main通过两图真实Nexa/GLM-OCR，顺序/各3锚点/性能/历史/重载保持，自有服务正常退出。最终包只因纠正README菜单名重打，执行文件与已验main保持一致。插件包在 `integrations/pi-desktop-ocr/dist/io.github.naza3.nexa-ocr-0.1.0.piplug`；此任务本地提交，不自动推送/tag，未修改Nexa二进制。用户Windows Electron完整安装、i5长图效果仍待验；准确步骤、产物摘要及分层证据见[插件说明](integrations/pi-desktop-ocr/README.md)、[ADR0038](docs/decisions/0038-pi-desktop-ocr-plugin.md)、[验证记录](docs/verification/2026-10-08-pi-desktop-ocr-plugin.md)。环境依赖刷新与保存草稿完成，未代用户发布环境快照。

## 2026-10-08 Windows 桌面 Clippy 修复与交叉便携包（交叉编译及原生CI已完成）

任务W05-CI-FIX-3 / W05-CROSS-2。[Actions37716280319](https://github.com/Naza3/Nexa/actions/runs/37716280319/job/113113404013)在Windows桌面Clippy失败：`windows.rs:1009`的`window.eval(&format!(...))`触发`needless_borrows_for_generic_args`，既有`-D warnings`将其提升为错误。该轮前端960项和Windows壳37项已通过；Runtime、下载组件与版本job成功，后续native打包跳过。按锁定Tauri接口改为直接传入`String`，关闭保存流程保持，未关闭lint。

本地主机fmt/Clippy退出0，独立审查核对锁定Tauri接口与关闭流程通过。用户明确要求先完成本地Windows交叉编译，检查通过才推送；已写入AGENTS。工具链已准备并复验，正式门槛脚本退出0：前端、189项原生编译、两workspace的Windows Release strict Clippy、四EXE实际链接和源码/原生库/PE身份核对全部通过。保留已有clang-cl工具探测warning。证据`/workspace/onboarding/windows-cross/runs/20261008T024028Z`；SDK冷准备17分49秒，缓存复验约0.5秒。

完成本地门槛后提交并推送`0ecddad0b837a9ea5571b01bc28eea73461357d1`。[原生Actions37719780731](https://github.com/Naza3/Nexa/actions/runs/37719780731)于03:15:07 UTC整轮成功：五个构建/验证job成功，分支Release正确跳过。Windows根Rust625通过/10既有忽略、桌面壳37、前端960、Python319项（317通过/2skip）、CTest5/5；真实模型、HTTP/CLI、桌面整包及安装器生命周期步骤通过。没有创建tag、发布Release或合并main，版本仍0.2.3。上述为原生CI构建的验证，不转授本地交叉包Windows运行结论。

用户要求下载本机产物后，在干净0ecddad上复跑交叉门槛退出0，结合本次同提交aria2与验证过的官方CRT生成`dist/Nexa-0.2.3-windows-x64-cross-0ecddad.zip`，18,721,201字节，生产打包校验及独立交付复核通过。启动路径`Nexa-Windows-cross-test/desktop-windows/nexa-desktop.exe`，需已有WebView2、自备模型；本地交叉EXE未在Windows执行，用户Win10/i5实际GUI、批量OCR与性能仍待实机验证。环境复用配置已保存草稿，需在环境设置中保存并发布后供后续环境使用。下一步为用户下载试用；详情、SHA与证据见[本轮验证记录](docs/verification/2026-10-08-windows-cross-gate.md)。

## 2026-10-08 按导入顺序批量OCR（源码与Linux验证完成，Windows待验证）

任务W02-OCR-6。一次最多20张PNG/JPEG、每张4MiB，默认保留导入列表顺序（不按文件名排序），开始前可上移/下移/移除。逐图准备和串行请求，复用已加载模型；正文保存成功后推进，每图独立结果/性能/100条历史。失败或停止暂停，恢复原请求不重放，明确继续仅处理未开始项。controller整批占位防图间被本窗口其他操作抢占；跨页继续，关窗停止调度并保存当前部分结果。原图/待运行队列不落盘，TOML偏好保持。

前端全量45文件958项通过，最后批量19项、互斥4项及旧OCR74项定向通过（子集不加总）；typecheck/lint/build与真实Chromium三尺寸模拟API检查通过。单图重跑迟到指标、外部模型变化、旧poll和正文超限边界已验证。没有Rust/协议/ACL/依赖变化，版本仍0.2.3；main已同步包含，本地提交不自动推送/tag。Windows原生文件窗口、WebView2及用户i5目标机仍待验证。见[ADR0037](docs/decisions/0037-sequential-image-ocr-queue.md)、[验证记录](docs/verification/2026-10-08-sequential-ocr.md)和[使用说明](docs/ocr-windows-cpu.md)。

## 2026-10-08 输出摘要、OCR历史与TOML设置（源码与Linux验证完成，Windows待验证）

任务W02-PERF-2 / W02-OCR-5 / W02-PREF-1。性能摘要放在聊天和OCR正文下方，严格匹配生成连接的真实实例与终态；最近100条非空OCR结果单独持久化，含不完整标记与可用指标，支持查看、复制、另存、删除，不保存原图。运行配置仍为config.toml，新增workbench-preferences.toml记忆OCR参数/提示词/缩放/视图及未发送聊天草稿；模型档案与本窗口草稿做基线冲突检查，不静默恢复初始默认值。

正常关闭先停止并消费OCR终态、保存已有正文与设置；标题栏通过固定事件/nonce ACK进入同一流程，未响应不默默丢弃。独立文件锁、CAS、原子替换及关闭写入门槛已验证。bridge155项、最后参数范围子集9项、Linux壳38项、包装ACL17项、fmt/clippy通过；前端全量936项、typecheck/lint/生产构建及三种宽度浏览器模拟API检查通过；最后定向检查与证据见[本轮验证](docs/verification/2026-10-08-output-history-and-preferences.md)。Windows原生构建/WebView2与用户i5目标机待验，版本仍0.2.3。见[ADR0036](docs/decisions/0036-desktop-results-and-preferences.md)及[OCR说明](docs/ocr-windows-cpu.md)。

## 2026-10-08 统一推理性能系统（源码与Linux验证完成，Windows待验证）

任务W02-PERF-1。用户要求从OCR扩展至所有推理。原生prepare/prefill/decode/同步输出回调计时随可靠终态传递，单actor保留最近200条内存历史，覆盖聊天、OCR、本机/LAN流式及非流式、实际生成短测。新增本机鉴权 `/runtime/performance` 与受ACL约束的 `performance_get`，实例绑定与有界校验；独立性能页按模型/状态/类型筛选，显示真实token速度、耗时与实际加载参数，支持复制CSV。失败/取消/超时不冒充成功测速；记录不含正文/图片/路径，服务退出清空。

完整Rust596通过/10既有忽略，Python314通过/5平台跳过，Linux壳34通过，前端全量879及随后定向19项、fmt/clippy/typecheck/lint/生产构建通过。真实Qwen文本、IPC4 worker信用取消、GLM-OCR真实HTTP计时/取消/恢复均通过；浏览器1440/1024/390布局、筛选、剪贴板及停服隔离通过（模拟后端）。私有worker协议升4，shim4与公共HTTP1保持；版本仍0.2.3。本地交付不自动推送或创建tag，Windows新包与用户i5目标机为下一步。见[ADR0035](docs/decisions/0035-unified-inference-performance.md)、[验证记录](docs/verification/2026-10-08-inference-performance.md)和[使用说明](docs/inference-performance.md)。

## 2026-10-08 可配置推理执行超时（源码与Linux检查完成，Windows待验证）

任务W02-OCR-4。新增“设置 → 资源与校验 → 推理执行超时”，复用既有执行秒数，默认300、停服可保存1–86400，重启服务生效。桌面聊天/OCR通过同一认证连接读取运行有效配置，响应头等待与流式等待同时跟随配置，移除这两处固定750秒限制；旧TOML兼容、CAS、取消、不重放和部分输出保留保持。浏览器发现并修复新设置草稿恢复白名单遗漏，补中文超时指引。

相关Rust258通过/2既有忽略、Linux壳34、前端868及最后preview定向18项、fmt/clippy/typecheck/lint/生产构建和独立审查通过；Chromium保存、启动应用、刷新恢复/冲突及1440/1024/390布局通过，后端为模拟。版本仍0.2.3，Windows原生新包和用户CPU长图仍待验。见[ADR0034](docs/decisions/0034-configurable-execution-timeout.md)、[验证记录](docs/verification/2026-10-08-configurable-execution-timeout.md)及[使用说明](docs/ocr-windows-cpu.md)。

## 2026-10-07 版本0.2.3与tag（已完成；公开Release受阻）

任务W05-VERSION-4。七版本文件一致性、锁定metadata及发行39项检查通过；`6309dedf6e15019a93f51ca24a27442850e7575d`与附注tag `v0.2.3`已原子推送，包含本轮全部修复。main仍1c3650c，未自动合并或移动旧tag。

[正式Actions37638109341](https://github.com/Naza3/Nexa/actions/runs/37638109341)的五个构建/原生/安装器job全部成功，前端850项、壳33项及13安装生命周期通过；Release job启动前GitHub报Internal server error，整轮failure，两次rerun-failed均HTTP500。六个原始发行文件已独立验证；复用生产发布流程补发时附件上传又遇401，已保留带正确manifest marker的未公开空草稿，未修改旧Release。可先下载[已验证的CI发行资产](https://github.com/Naza3/Nexa/actions/runs/37638109341/artifacts/11492458105)。具体来源、摘要、错误与恢复条件见[版本验证记录](docs/verification/2026-10-07-release-version-tool.md#023-原生结果与发布阻塞)。

## 2026-10-07 OCR 选图无预览（实现与原生构建已完成）

任务W02-OCR-3。修复有效PNG/JPEG被空或错误File.type误拒；用户实机具体MIME仍未知。现按内容识别图片，预览旁显示文件名/准备/错误，统一“选择图片”按钮，正确处理同文件重选及旧异步结果。最终前端850项、lint/typecheck/生产构建、真实Chromium九组格式组合、file chooser、缩放及三尺寸布局通过；0.2.3 Windows桌面和整套产物验收通过。浏览器回复为模拟，用户Win10 WebView2及实际OCR质量仍独立待验。详见[验证记录](docs/verification/2026-10-07-ocr-image-preview.md)。

## 2026-10-07 Windows CI 构建优化（原生验证已完成）

任务W05-CI-PERF-1。npm/Cargo缓存、Release依赖复用、前端去重、桌面/Runtime并行与冷缓存metadata修复完成。843的[Actions37632857580](https://github.com/Naza3/Nexa/actions/runs/37632857580)整轮成功，等待29分02秒，相比旧成功基线36分56秒少7分54秒（约21.4%），是桌面缓存命中/Runtime冷构建的一次观测。0.2.3 tag实跑确认五个Cargo save跳过、两套npm只恢复、tag缓存仍0条；main当前无缓存，尚无实际tag命中main的证据。发布平台错误单列，不计入成功加速。见[ADR0033](docs/decisions/0033-parallel-windows-builds.md)与[验证记录](docs/verification/2026-10-07-ci-performance.md)。

## 2026-10-07 产品版本 0.2.2（已完成版本同步，构建待验证）

任务 W05-VERSION-3。按用户要求，在包含 OCR 配对与布局修复的 `e5741ac` 基线上，使用一键工具同步七个版本文件至 `0.2.2`。版本检查、重复预览零修改、两个 workspace 的锁定离线 Cargo metadata、版本/发布测试39项（38通过/1平台跳过）及独立差异审查通过，第三方依赖保持。具体命令与证据见[版本更新记录](docs/verification/2026-10-07-release-version-tool.md#产品版本更新至022)。本任务只更新并推送开发分支，版本 tag 与正式发布由维护者操作。

## 2026-10-07 OCR 配对与页面布局（源码及本地检查已完成，Windows 待验证）

任务 W02-OCR-2。用户确认主模型选完后没有第二个窗口，定位为 Windows canonical 路径被误用于外部文件选择与导入。已保留原始对话框路径用于保护/导入，canonical 仅比较；补上导入区域等待、取消、错误和成功反馈，以及成功后模型列表刷新和旧读取失效。

按用户追加要求，页面沿用 Nexa 全局风格，分模型准备、图片设置、结果三区，宽屏并排/窄屏堆叠。完整前端818项（含针对性45项）、Linux壳34项、严格Python297项（292通过/5平台跳过）、静态检查/前端构建与独立审查通过；浏览器1440/1024/390px模拟预览无横向溢出。Windows专用路径回归已加入既有CI，原生新包与目标机操作仍待验证。详见[本轮验证](docs/verification/2026-10-07-ocr-pair-and-layout.md)和[更新后的操作说明](docs/ocr-windows-cpu.md)。

## 2026-10-07 GitHub Actions 原生 Node.js 24 升级（本地检查已完成，原生 CI 待验证）

任务 W05-CI-NODE24-1。按用户要求，将四个自有工作流中 27 处 checkout、setup-node、upload-artifact、download-artifact 调用升级到官方原生 Node24 版本并固定完整 SHA。显式保持 ZIP 归档、关闭新版自动 npm 缓存，并采用下载摘要不符即失败；既有触发、权限、产物名称及版本门禁保持。

actionlint、配置结构对比、严格完整 Python 297 项（292 通过/5 平台跳过）及独立审查通过。保留产品 `0.2.1` 和 Node/npm 工具链；精确提交原生执行仍待验证。版本、兼容性与实际命令见[本轮验证](docs/verification/2026-10-07-actions-node24.md)。

## 2026-10-07 公网 TLS 检查有限重试（Windows 组件与脚本检查通过，完整构建中）

任务 W05-CI-NET-1。用户要求避免公网证书样例偶发断连/超时反复打断构建。开发分支已同步 main `02c90da`，保留产品 `0.2.1`。新增仅针对四个固定公网样例的三次上限、2/5秒退避和独立目录；每次沿用原通过判定，重试耗尽与非网络故障仍失败，报告保留全部尝试并在日志显示原因。

两次失败 Actions 的三条真实诊断（一次连接重置、两次超时）重放均只授予重试资格，原失败判定保持；严格完整 Linux Python 297 项（292 通过/5 平台跳过）通过，独立31项日志反例和5种控制场景复核通过。精确提交 `c8163f452192c06e1271945c43577562b3c1d21c` 已推送，[Actions37614473400](https://github.com/Naza3/Nexa/actions/runs/37614473400) 的真实 Windows 组件策略门禁及完整 Python suite 所在步骤均通过，后续完整构建仍进行中。

生产 aria2 补丁/输入锁、TLS 参数、应用与版本文件均未修改；没有移动 `v0.2.1` tag。仅补记结果的文档提交不改变上述验证源码身份。详见[本轮验证](docs/verification/2026-10-07-public-tls-retries.md)。

## 2026-10-07 Windows 版本工具测试路径修复（原失败步骤已通过，完整 CI 进行中）

任务 W05-VERSION-2。[Actions37603689299](https://github.com/Naza3/Nexa/actions/runs/37603689299) 在 Windows Python suite 的版本工具测试失败：快照键使用平台原生反斜杠，与七文件清单的正斜杠字符串比较，误报修改库存不同。唯一测试失败已经日志确认；Windows cmd 入口及其它版本工具测试未失败。修正快照为 `relative_to(...).as_posix()`，保留实际文件与内容比较。

同步用户已合并并升级的 main `997117b`，产品版本保持 `0.2.1` 且一致性检查通过；Linux 完整严格 Python 288 项（283 通过/5 平台跳过）通过，Windows/POSIX 路径反例复现旧比较差异并验证修复。修复提交 `cdbc12a98dfe7b4f596d77f22f25ef8a06ae86dd` 已推送，其 [Actions37605103921](https://github.com/Naza3/Nexa/actions/runs/37605103921) 第二次尝试中，下载组件门禁与包含完整 Python suite 的原失败步骤已成功，后续完整构建仍进行中。

首次尝试在外部 `self-signed.badssl.com` 证书样例超时，报告为31/32通过；同提交仅重跑失败 Windows job 后该门禁通过，未修改 aria2 源码或放宽证书校验。[补充验证](docs/verification/2026-10-07-release-version-tool.md#第二轮windows路径修复)区分本地与原生结果。随后仅补记验证的文档提交不冒充本次产物来源；没有移动 tag 或发布 Release。

## 2026-10-07 一键更新版本与 CI 修复（源码及 Linux 验证已完成，Windows 待验证）

任务 W05-VERSION-1。用户要求修复 Actions 构建失败，并提供一次更新全部版本文件的工具。`codex/dev` 已同步最新 main `2abb7bef7329032fe755f19dc3605c863949cb4c`，保留用户选择的 `0.2.0`，补齐遗漏的 npm 锁文件版本。

- 根目录 `update-version.cmd` 提供 Windows 双击输入及命令行入口；`scripts/set_version.py` 使用 Python 3.11+ 标准库离线同步七文件，支持 `--dry-run`、`--check`、已有错配修复及普通写入错误回滚，保留第三方依赖与 LF/CRLF；不会创建提交或 tag
- 修复发布测试误改第三方同版本包的问题，生产版本门禁保持；真实开发分支 [Actions37599775078](https://github.com/Naza3/Nexa/actions/runs/37599775078) 另因漏编译 `air-ocr-template-test` 失败，已补固定构建目标并增加 CTest 目标覆盖回归
- 严格编码完整 Python 288 项（283 通过/5 平台跳过）通过；两 workspace 离线锁定 Cargo metadata 版本均为 0.2.0；实际固定原生目标在 Linux 增量编译成功，CTest 5/5 通过。Windows cmd 入口测试已加入现有 CI，在 Linux 跳过

使用方法见[版本更新说明](docs/windows-releases.md#版本规则)，证据与限制见[本轮验证](docs/verification/2026-10-07-release-version-tool.md)。Windows 原生新构建、安装器及目标机仍待验证；没有移动 tag、合并 main 或发布 Release。下一步是推送开发分支运行既有 Windows CI，由维护者在包含修复的提交上发行。

## 2026-10-07 本机单图 OCR（源码与 Linux 开发验证已完成，Windows 待验证）

任务 W02-OCR-1。用户要求现有 Nexa 在 Windows10/i5-8400/16GB 纯CPU上传图片得到Markdown，并明确按实施顺序推进。`codex/dev`已快进到最新main `c559d7fc6a85313a46cd901868dcc1dd43446a5b`，保留最新模型移除与安全Markdown。固定llama提交已支持GLM-OCR，未升级引擎；先跑官方Q8双GGUF真实基线，再实现配对托管导入、CPU mtmd/worker图片路径、本机HTTP与桌面OCR。

- [ADR0032](docs/decisions/0032-local-single-image-ocr.md)：单user/单PNG或JPEG/单提示词，保留图文顺序及真实图像token预算；LAN文本、现有单actor/取消/worker隔离保持。双文件hash/结构/身份一起校验，旧单文件零复制保持
- 新增OCR页面、双原生选择、可选图片缩放、显式8192/256/4加载参数、流式原文/安全Markdown、复制和原生保存。读取异常保留任务直到终态确认，读取缺批次和length截断明确提示；不会自动重生成
- 私有worker3/shim4、公共HTTP1；静态闭包10库含mtmd/vendor-hash，新内嵌许可完整打包。配对仅报告Loaded，旧文本Passed不升级为OCR证明
- 最终根Rust45组576通过/0失败/10忽略，前端38文件809项，Python275项（271通过/4平台skip），Linux壳32、CTest5/5，相关fmt/clippy/typecheck/lint/build全部通过；分层子集不重复加总
- 真实GLM配对导入/关闭重开、native坏图/预算/取消恢复、HTTP SSE三行/控制取消/非流式恢复通过，JPEG补测三行通过；固定Qwen原adapter3项和engine-host1项真实文本回归通过。复杂官方页只4/5锚点的质量失败完整保留

开发环境为Linux Xeon8573C/4核配额，不是用户目标机；没有本批Windows原生编译、GUI手验、安装包、i5速度或官方网站复杂版面质量结论。独立审查发现的旧projector复用及三处UI恢复/完整性问题已修复并复验。详见[本轮验证](docs/verification/2026-10-07-local-ocr.md)与[Windows CPU使用说明](docs/ocr-windows-cpu.md)。修改仅本地提交，未推送/合并/发布；下一步是精确源码的原生Windows构建和目标机验收。

## 2026-10-06 模型移除与聊天 Markdown（已完成：原生验收及开发包交付）

用户要求从模型库列表移除模型，并补充聊天回复 Markdown。开发沿用 `codex/dev`，已在本地同步用户合并的正式 `main` `5c26e34aa74bf5552b9455e65f42883acbd61d6f`，保留开发分支额外验收文档，不重写历史。既有 [v0.1.0 正式 Release](https://github.com/Naza3/Nexa/releases/tag/v0.1.0)已于2026-10-05发布；本轮不移动其标签、不替换公开附件或自动合并 main。

- [ADR0030](docs/decisions/0030-nondestructive-model-unregistration.md)：只取消模型登记，保留GGUF/manifest、参数档案和历史记录；schema3同文件原子抑制，重启不复活，显式添加/扫描可恢复。在线由actor互斥，驻留先卸载、故障先显式停服，不打断其他调用；离线不启动服务或hash全库
- [ADR0031](docs/decisions/0031-safe-chat-markdown.md)：模型回复安全Markdown/GFM展示，保留原文和请求语义，禁原始HTML执行、图片自动联网及WebView导航；实现与联合前端回归进行中

模型移除后端四crate381通过/0失败/1既有忽略，壳Linux31通过，相关clippy/fmt通过；列表整锁调整后model-store88项与clippy复验。最终联合前端36文件790项、typecheck/lint/build全部通过。Python271项（267通过/4平台skip）通过，新增IPC命令保持精确ACL闭包。独立移除审查Rust5项/前端7项、Markdown50项恶意内容/流式/错误隔离回归全部通过；这些为分层或子集证据，不累加成产品用例总数。

发现并修正：Faulted服务不能卸载，移除指引改为用户显式停服；新嵌套npm依赖许可输出原路径被安全门禁拒绝，改用锁路径hash命名并保留lock_location归属，不放宽底层校验；深嵌套Markdown解析异常由单消息边界回退原文，保持会话和复制。107生产包109许可/110原件逐字节恢复，结合既有包许可验证仍10份文件，无许可删减。JS生产包约601kB、gzip约179kB，Vite大chunk提示保留，没有借此扩大功能或引入高亮引擎。

精确代码提交 `09b9e0496f565e0e62859be2c7034e9ceb756325`、tree `bd33eb60826c40f6517b1beb3124afa8100f1645` 的[Windows CI37449560591](https://github.com/Naza3/Nexa/actions/runs/37449560591)于2026-10-06 11:08:26 UTC成功。Windows根Rust53组561通过/0失败/7忽略、壳29、前端790、Python271项（269通过/2skip）、CTest4/4，以及真实模型/HTTP/CLI/独立解压桌面和13项安装生命周期通过。分支Release job正确跳过。

独立实际产物2659项断言全部通过：同28文件payload、48 IPC/权限、EXE内实际Markdown JS/CSS、10许可文件恢复849原件及107包/109npm许可归属、446源码Windows指纹、aria2三补丁、54证据与69安装诊断事件闭合。开发便携包16,256,952字节，SHA256 `0ebe57bf38cc5a2ec0c0298be2021773abe178b8a7adec210e4ec22c2245d843`；[下载入口](https://github.com/Naza3/Nexa/actions/runs/37449560591/artifacts/11409385193)已于11:20:05 UTC发给用户，有效期2026-10-13，不能据此认定用户已安装。

本批版本仍0.1.0，仅为精确提交开发产物，未覆盖正式Release。MSI ProductCode与原正式版相同，13门槛中的升级/回滚使用私有未来版本fixture，不代表旧公开0.1.0到本批同版本包已验证升级；交付建议使用便携版。便携版不自动隔离既有数据，用户已被提醒停服并备份数据目录，schema3不能直接由旧版读取。目标Windows10窗口/干净机器/离线/长期条件仍独立待验。详见[本批验证记录](docs/verification/2026-10-06-model-unregister-and-markdown.md)。随后纯文档记录提交不改变本批产物源身份，不重跑完整构建。

## 2026-10-05 桌面源码清理（已完成：原生验证与产物复核通过）

安装器前置任务已完成并交付后，按用户明确要求移除本项目移动代码及相关文档，后续只维护桌面主线。清理基线为长期 `codex/dev` 的干净 `de7732f031c11e44a27f86b33a341c48131a3906`。删除独立移动 workspace、验证器、MNN 适配/探针/补丁、专用脚本与 CI，以及对应设计/研究文档；同步共享配置、验证报告字段和 Windows 路径过滤。范围见[ADR0029](docs/decisions/0029-desktop-only-source-tree.md)，本批实际检查见[清理验证记录](docs/verification/2026-10-05-desktop-only-cleanup.md)。

清理已提交并推送为 `f577a49861298ec278293ba2d7231e09c3c07022`，tree `f3ec548cc3049da7e7121d4866bcff07e360dfd4`。[Actions37313974388](https://github.com/Naza3/Nexa/actions/runs/37313974388)已于13:44:46 UTC成功：release-identity、同源aria2和native三项job成功，分支构建的Release job按条件跳过。Windows Server 2022上的根/壳Rust fmt、test与clippy、真实GGUF、HTTP/CLI、桌面bridge、三格式打包及全部13项安装生命周期通过。

本次原生证据的精确计数：根Rust日志53组551通过/0失败/7忽略、独立桌面壳29通过、前端33文件701项通过、Python270项（268通过/2个平台skip）、CTest4/4。早期选择性回归不重复加总；本地清理时Python266通过/4skip是另一环境的结果。开发阶段99份Markdown/415本地链接/19锚点与独立11组源码审查已通过，具体命令和分层结果见[清理验证记录](docs/verification/2026-10-05-desktop-only-cleanup.md#f577a498精确提交原生windows验证)。

本批[三格式Actions产物](https://github.com/Naza3/Nexa/actions/runs/37313974388/artifacts/11348344567)已通过独立Linux只读审计：2238项断言全部通过、0失败、无剩余产物阻断。这是本批新计数，含逐文件/许可检查，不是2238个独立产品功能测试。Setup内嵌MSI、MSI17张表与CAB28文件/42408929字节、独立portable/runtime、10许可恢复746份原文、aria2三补丁源码重放、54份原生证据及13项生命周期69事件均核验；435份源码重建Windows指纹与包来源一致。此处记录构建与复核完成，向用户交付另据实际发送记录。产物源身份始终为f577a498；随后仅补记状态的文档提交不冒称经过本次原生验证，也不为文档更新重跑整套构建。用户已接受当前约30分钟完整构建流程，暂不进行优化。

既有Windows actor/worker、GGUF/API、安全、用户数据及完整许可/对应源码边界保持；独立 `Naza3/MNN`、锁定llama.cpp上游完整源码和Git历史不动。原先保留移动源码、隔离CI、B3b WIP的要求已由本次明确删除授权替代。安装器未签名，用户Win10/i5-8400、应用原生GUI、ICE、两机LAN、干净机器/离线、长期条件与真实tag发布仍未验；没有合并main或发布Release。

## 2026-10-05 三格式安装器完成并交付

精确提交 `de7732f031c11e44a27f86b33a341c48131a3906`、tree `afde24f2d64c81cc7e4484c6c8641c7ed87a8fe1` 的[Actions37306309927](https://github.com/Naza3/Nexa/actions/runs/37306309927)已成功：Windows Server 2022 上真实模型/HTTP/CLI/桌面 bridge、MSI/Setup 构建及全部13项安装生命周期门槛通过。独立 Linux 只读产物审计2419项断言通过、0失败，包含逐文件/许可断言，不是2419个独立产品测试。

版本0.1.0的 Setup EXE、MSI、便携ZIP及校验/来源说明已于12:49:24 UTC交付。三格式同一28文件payload、10份许可闭包及aria2对应源码核验通过，Setup内MSI、MSI CAB与便携字节完全相符；623份源文件重建Windows CRLF指纹与原生清单一致。准确大小/hash、13门槛与未验证项见[最终交付证据](docs/verification/2026-10-05-tag-release.md#最终de7732f原生成功与三格式交付)。

[PR #9](https://github.com/Naza3/Nexa/pull/9)仍为草稿，`main`仍为`4c40a0d0f969dfb6d10a295bb7b7922f741c9643`。本次是分支构建，Release job按条件跳过，未创建/推送实际tag、未发布Release或合并main。交付未签名，ICE未运行；用户Win10/i5-8400、应用原生窗口、两机LAN、干净机器/离线及长期稳定性仍独立待验。Setup向导CI通过不能转授应用GUI或目标机结论。

## 以下为桌面阶段历史

以下条目保留其当时的提交、失败、待验和交付状态；现状以上方最新条目为准，不将旧验证结果追溯覆盖新源码。

## 2026-10-05 安装维护范围不变量核验（待原生验证）

`4067823`的[第四轮37300524739](https://github.com/Naza3/Nexa/actions/runs/37300524739)已通过真实MSI早期门禁、默认安装及登记/字节校验、MSI与Setup修复、短路径修复、实际runtime启动、忙进程阻止维修/卸载和正常停服。旧查询数字已确认stage7/major6/minor3/build20348/error1150：版本被返回为6.3后拒绝，不是API读文件失败。新失败为已安装per-user产品维修时传ALLUSERS=1返回0，而测试硬编码1603；不能仅凭0认定范围升级。候选修正以官方API证明真实上下文仅当前用户USERUNMANAGED=[2]、无machine/managed登记、原组件路径/逐文件hash/数据哨兵不变且不创建外部目录，再判断维护请求是安全归一化或拒绝；首次安装覆盖范围/目录仍必须1603，生产C门禁不改。私有3010测试MSI修改后刷新并读回PackageCode。严格Python265项（261通过/4skip）及独立安全不变量/发行回归通过；下一原生运行仍需完整升级/回滚/降级/卸载/3010/向导门槛。移动清理保持后置。

## 2026-10-05 MSI系统检测拒绝的候选修正（待原生验证）

`2bbd72d`的[第三轮37291142335](https://github.com/Naza3/Nexa/actions/runs/37291142335)通过701项前端及其后真实模型/便携包门槛，安装生命周期在默认MSI入口超时。新增诊断已证实最后动作NexaGuard、窗口原因guard_os；向导取消、非法参数及未安装卸载分支通过。旧kernel32文件版本查询的具体失败子步尚无证据，不将失败边界等同完整根因。现在将系统谓词移到带Win10兼容manifest的独立只读EXE，以同步MSI Type2动作强制检查返回码，再进入原目录/忙进程guard。长构建前新增真正System32/msiexec的无安装只读探针，要求生产OS/guard动作成功且产品仍未注册、Nexa目录未创建，并记录旧查询的数字阶段/版本/错误。全部13项安装门槛、默认UI和原超时不变；严格Python259项（255通过/4既有skip）、独立契约/诊断复核通过。候选修正等待下一精确提交原生结果，移动清理仍后置。

## 2026-10-05 安装器第二轮CI被LAN回归拦截（修复后待重跑）

`8ff6447` 的[第二轮37288485584](https://github.com/Naza3/Nexa/actions/runs/37288485584)已通过版本门禁、同源组件与安装器早期编译/路径检查，但前端695项中一项LAN保存测试失败，尚未进入安装生命周期，不能判断MSI候选修正效果。确定性红测分别证实测试mock保存后未更新后端状态，以及兼容LAN保存ACK与action finally之间旧读取可覆盖新快照的微任务窗口。仅在该ACK写入前同步推进snapshotEpoch，保留保存后的新后端读取权威性；统一configuration路径不改。完整前端701项、typecheck/lint/build通过，10轮60项聚焦回归及独立Node微任务8组对照通过，见[LAN时序验证](docs/verification/2026-10-05-lan-save-ci-race.md)。安装器与13项门槛不变，等待新精确提交原生CI；移动清理继续后置。

## 2026-10-05 Tag 三格式自动发行（进行中）

用户要求 GitHub tag 自动构建便携版、MSI，并追加 Setup 安装包。本轮在 `codex/dev` 的精确 `9a3de0317129d0c09f6986a0e758022f2f83ea21` 基线上保留已有原生 Windows 全门禁，增加严格稳定 tag/版本一致性验证、同一已验证 payload 的三格式打包与安装生命周期，以及仅 tag push 可进入的隔离 Release 发布。见[发行说明](docs/windows-releases.md)。 用户已于07:32:53 UTC合并[PR8](https://github.com/Naza3/Nexa/pull/8)，main为`4c40a0d0f969dfb6d10a295bb7b7922f741c9643`且tree与9a3完全一致；开发分支已快进同步该main后继续本批，不重写用户合并。不自行选择发行版本，不创建 tag、合并 main、修改仓库安全设置或新增签名服务。

首轮源码严格 Python 247 项通过；本次诊断修复工作树复验全量 255 项（251 通过/4 既有平台 skip；发行 27 项、安装器静态 10 项及诊断 6 项均为子集）、actionlint 1.7.12、实际旧 Windows ZIP 跨平台 28 文件身份复验及 diff 检查通过，见[开发验证](docs/verification/2026-10-05-tag-release.md)。开发验证和原生安装器验证分别记录；首轮精确 `51d2d506a0bc4888b84bab5fa7043a52806d4dac` 的 [Actions37281440005](https://github.com/Naza3/Nexa/actions/runs/37281440005) 已完成应用/原生真实验证与 MSI/Setup 实际构建，但安装生命周期等待 240 秒后失败，整体未通过、未发布。现增加封闭脱敏的逐阶段诊断和失败报告保留以定位，不能在缺乏该报告时断言具体阻塞窗口或降低门槛。当前没有本轮完整 Windows CI 成功结论，不将已有 9a3de03 便携包证据转授安装器。现有 28 文件/10 许可闭包与对应 aria2 源码必须完整保留，卸载/升级必须保留模型和配置；目标 Win10 GUI、干净机器/离线/长期条件仍独立待验。移动端清理是用户随后提出的下一任务，本轮不混入删除。

## 2026-10-05 按操作身份手动停止加载（开发验证完成，Windows待验）

用户要求加载耗时长时可以手动停止。在 `codex/dev` clean基线 `a5ba7388d23758d31e2bfbc94571ef906fd8e535` 实施[ADR0027](docs/decisions/0027-owned-model-load-cancellation.md)：模型库legacy/profile、添加/下载后的自动加载共用按次UUID，取消覆盖hash/切换/native load/本操作私有短测；不停止整个服务、不取消别的客户端、只在清理ACK后终态，旧令牌不能影响新工作。生成与LAN管理边界保持，已保存/登记不回滚。

最终全workspace/all-targets42组539通过/0失败/7既有忽略，另doc-tests7；完整clippy/root与壳fmt、壳31项/clippy、Windows壳及关联后端all-targets交叉check（仅既有clang-cl探测warning）通过。前端695项/typecheck/lint/build、Python208通过/4平台skip通过；独立96项与最终稳定进程fixture复验为子集不累加，审查无剩余阻断。真实不合作Load在收到Load后的原子标记取消，5.07秒kill/reap后同supervisor正常重载；坏IPC/原生真实故障不再被Stop覆盖为正常取消。最终重建Linux固定Qwen0.6B实际loading阶段Stop，断言终态cancelled、无active/registry工作及旧ID无效，随后同服务重载、短测/聊天/证明与最终清理全部通过。详见[验证记录](docs/verification/2026-10-05-model-load-cancellation.md)及[脱敏真实报告](docs/verification/2026-10-05-model-load-cancellation-smoke.json)。

本轮子任务没有提交、推送或触发Actions；新原生Windows/目标机结果尚无。由主代理统一提交并按当前授权运行标准GitHub Windows构建，不自动合并main，不把交叉check冒称Windows运行或新包交付。

下文为以前阶段快照，不覆盖本节新任务与验证状态。

## 2026-10-05 许可无损整合与整体桌面精简（进行中）

当前源码基线为`codex/dev`的`a14eb6fdb1858baf507c8b9a8509b0ca30df1316`；最后实际交付仍为a14原Windows包，[Actions37216837409](https://github.com/Naza3/Nexa/actions/runs/37216837409)已成功。本节更新当前状态，下文保留各阶段历史，不将旧包CI证据转授本次源码。

L01许可整合源码及针对性验证已完成：完整桌面目录含嵌套runtime/download至多10份许可文件，原生与已有交叉库存均为4+4+2，全部原文/NOTICE/版权HTML/原库存字节和来源映射保留。Microsoft原DOCX/PDF独立保留时，只把本层root notice完整并入文本。两份旧ZIP临时语料分别恢复748/760份原文，12个非许可payload及对应源码完全不变；未生成新ZIP。严格Python212项（208通过/4平台skip），Rust download-engine与xtask针对性77通过/1既有忽略、同范围clippy及格式检查通过；独立审查发现的验收器归属校验缺口已修复并实际执行反例回归。见[ADR0026](docs/decisions/0026-lossless-license-bundles.md)及[验证记录](docs/verification/2026-10-05-lossless-license-bundles.md)。

整体桌面精简源码已冻结，前端654项测试、typecheck/lint/Vite构建通过；独立UI审查14项反例及88项永久回归子集通过，四处焦点/跨页失败反馈/过期证明状态问题均已修复。许可独立审查的伪归属与超界整数精度反例亦已关闭，无剩余源码阻断。产品方案见[精简桌面体验](docs/product/compact-desktop-experience.md)，本轮过程见[桌面验证记录](docs/verification/2026-10-05-compact-desktop-experience.md)。用户02:34:54 UTC最新要求“这批修改完成后github构建”，覆盖此前暂缓构建：本批源码验证及独立审查已收口，现在由主代理统一提交、推送并运行标准GitHub原生Windows构建；本次尚无新CI或新包结论。Win10窗口、两机LAN及离线/长期测试仍独立待验。

## 2026-10-04 整体产品体验实施（进行中）

四项修复6aa0e1f已通过[Actions37206035656](https://github.com/Naza3/Nexa/actions/runs/37206035656)并交付完整Windows包；[PR8](https://github.com/Naza3/Nexa/pull/8)仍未合并。随后完成整体流程审查与12页设计，用户15:05 UTC明确批准按方案实施。当前在同一codex/dev落实[整体体验契约](docs/product/experience-implementation.md)，先状态/配置/CAS与模型档案，再页面及任务流程，保持单actor、安全边界与原接口兼容。本批源码已冻结，联合Rust507/0/7、前端610、Python190/4skip、clippy/fmt及Windows交叉check通过；独立前后端审查无剩余阻断，Linux固定真实GGUF的档案/CAS/重载/空闲恢复/空model/SSE及坏配置停服13项通过。详见[实施验证](docs/verification/2026-10-04-unified-product-experience.md)。现在进入统一提交与GitHub原生Windows构建，尚不宣称新Windows包已通过。main仍e3c5，无自动合并或新功能分支。

## 2026-10-04 四项修复统一提交与Windows构建（进行中）

用户13:28明确要求“修复完成后再提交，在GitHub上构建”，解除下文13:01起暂缓安排。添加结果关闭、LAN网卡候选选择、侧栏统一服务主控、空model默认当前加载模型四项源码与独立审查已完成；联合Rust482通过/0失败/7既有忽略、前端508项、严格Python190通过/4平台skip、clippy/fmt及必要Windows交叉检查通过。现在在 `codex/dev` 统一提交并使用公开仓库标准Actions原生Windows构建；确切run与产物按提交后结果记录，尚不宣称新Windows包已通过。最后交付仍为e0ff1e6。main未自动合并；HTTPS、GPU和复制API ID不在本批。

## 2026-10-04 空模型ID默认当前加载模型（源码验证完成、暂缓构建）

用户明确确认“空模型ID就使用当前加载的模型”。按[ADR0024](docs/decisions/0024-current-loaded-model-chat-default.md)实现缺省/空串/全空白选择当前Ready/Generating模型，actor原子绑定并返回实际响应ID；无模型不加载，显式ID不回退/自动切换，null与其他类型仍非法。现有本机首次显式加载/同selected重载及LAN只允许本机已加载模型保持。仍按用户要求暂缓提交推送触发CI及新包，仅进行本地源码实现/验证。复制ID按钮仅为建议，未加入本轮实现。最终联合Rust482通过/0失败/7既有忽略、完整clippy/fmt通过；core/API独立133项子集审查通过，无阻断。详见[本轮验证](docs/verification/2026-10-04-current-model-api-default.md)。尚未重新运行真实GGUF/原生Windows，不将本地通过称为新包已交付。

## 2026-10-04 桌面控制与本机网卡选择（源码验证完成、暂缓构建）

在长期 `codex/dev` 按用户最新反馈改进三处交互：添加结果可关闭、局域网IPv4自动列出网卡供选择、左导航栏底部统一启动/停止服务主按钮。HTTPS明确暂缓；地址发现是本机只读操作，不自动启用服务或放宽网络配置。联合前端508项、Rust全workspace/all-targets472通过/7既有忽略、完整clippy/fmt、严格Python190通过/4平台skip及Windows交叉check通过；独立Rust/UI审查无阻断。原生Windows与新包尚未执行，详见[本轮记录](docs/verification/2026-10-04-desktop-controls-and-lan-discovery.md)。最后已交付仍为下文e0ff1e6包，不将开发中的功能称为已交付。用户随后反馈API缺失模型ID返回400，要求先不着急构建；当前暂停新包/Actions触发，仅继续本地回归及只读行为诊断，未改空ID或自动切换语义。

## 2026-10-04 本批交付完成与长期分支切换

最终源码 `e0ff1e6cbb03fde6ae91a5f7272cd73d62b3f1ce` 的[Windows Actions37199537016](https://github.com/Naza3/Nexa/actions/runs/37199537016)已全部success。原生Windows真实模型、HTTP/CLI、模型下载自动登记、managed/external加载与重复短测、停服离线记录及完整提取包验收通过。54证据文件和最终包独立字节/PE/许可/对应源码复核通过；Win10用户GUI/选择器/剪贴板、两机LAN、干净机器/离线/长期稳定性仍独立待验。

完整桌面包已交付：16791161字节、766文件，SHA256 `ef74e136acde2e381254dd0b8f191a9fe397d9b1ccac938a774713253ffbcf63`。其源身份始终为e0ff1e6，不改写为后续文档或合并提交。按用户明确请求，[PR #7](https://github.com/Naza3/Nexa/pull/7)已合并main，merge `e3c5cf2658ed8501c74466f2533d13c56f45edf7` 与包源tree完全相同。

用户最新指定以后从main统一使用 `codex/dev`。已从上述最新main创建该分支；后续功能均在此推进，交付前同步main并处理冲突。旧 `codex/nexa-add-model` 仅保留历史，不再作为后续开发入口。本次只同步分支名/CI触发与规范、状态，不改变产品代码；纯配置提交明确跳过重复整包CI，后续功能提交仍正常触发标准Actions。

## 2026-10-04 Actions第二轮短路径修复（待新CI验证）

[run37198513508](https://github.com/Naza3/Nexa/actions/runs/37198513508)，head `82bd70fcd40c03760d59ed9850443e1dbbd97ffb`：Ubuntu同源组件成功，Windows下载probe实际32/32通过（3数字别名为resolver提前拒绝、4非法URI为明确DEBUG解析拒绝，均不宣称socket gate执行）。Rust1.98.1、CMake4.4.4、VS2022/MSVC14.44.35207及源身份准备成功。

随后Windows严格Python188项出现5fail/6error/1skip，全部为VS/CRT测试中短路径RUNNER~1与canonical长名runneradmin混用的relative_to误拒。实际来源比较修复已完成：先检查原路径及祖先，再统一真实路径表示；保留同VS/Release/x64/版本门槛，未只改fixture或跳过测试。严格Linux Python194项（190通过、4平台skip）、py_compile/diff已通过，新Windows8.3/junction用例待下一CI。日志末尾Security模块重复成员是既有隔离测试的预期诊断，不是这次失败原因。Nexa编译和整包仍未进入，继续同一分支/PR。

## 2026-10-04 Actions首轮Windows探针修正（进行中）

公开库标准runner已实际分配：[run37197414719](https://github.com/Naza3/Nexa/actions/runs/37197414719)，精确head `cbba057b0106b7cc65131332858c0cedc993ff93`。Ubuntu同源aria2构建成功；WindowsServer2022在早期下载组件probe失败，32case中25通过、7失败，尚未进行Nexa/Rust/CMake构建。68policy、26Request/4socket、53payload以及公开HTTPS和三类错误证书拒绝已在该Windows运行中通过，不代替整任务成功。

该run三个特殊数字私有地址观测到resolver在socket gate之前失败；四个非法URI原Windows仅有resume提示，源码分析及同源Linux单例观察指向Request::parseUri后的无URI debug分支，Windows debug证据待新run。探针分类/诊断修复已完成，严格Python188项（186通过、2平台skip）及独立37项子集通过，只修改测试分类/诊断，不改生产补丁、来源锁或TLS规则，不将任意DNS/非零退出当通过；原32case仍须下一轮实际执行。继续同一开发分支及PR #7，不改main。

## 2026-10-04 公开仓库恢复GitHub Actions（进行中）

用户明确将仓库改为public并恢复后续GitHub Actions构建；GitHub API已确认visibility=public。此要求覆盖下文旧“不运行Actions Rust”的约束，但不授权收费runner或付费资源。继续维护codex/nexa-add-model，PR #7已经建立且在2f7478f时与main26206ef无冲突。

本批功能源码464项Rust、414前端、Python153+2skip和Linux真实模型已验证，2f7478f四Windows交叉EXE与同源aria2也已完成；其完整ZIP未产生，因为微软CRT在线CRL访问被云端策略阻断，正式提权又在命令前沙箱挂载失败，用户再授权重试仍同样失败。未跳过校验，旧交叉组件保留但不能称为可交付包。现在按用户新要求适配原生Windows Actions，进行同源组件重建、完整验证和打包，不继续重试原云端受阻网络。恢复改动已完成严格Python170项（168通过、2平台skip）、YAML/py_compile/diff静态检查，见[恢复记录](docs/verification/2026-10-04-public-actions-restoration.md)。尚未触发本轮CI，实际运行/产物结果以精确head SHA后续记录，不能把静态通过称为Windows通过。

## 2026-10-04 校验超时与空闲策略（源码联合回归完成）

Windows本机记录/反馈修复已在同一长期分支推送 `abcb1a0a9b448915cf311e4cf33c427cf6af017b`，449项Rust加4项doc、343前端、Python153+2skip和Windows交叉检查通过。按[ADR0023](docs/decisions/0023-model-verification-and-idle-policy.md)的两设置已实现并冻结，仍在同一开发分支。父全workspace/all-targets464通过/0失败/7忽略、clippy/fmt、前端414项/typecheck/lint/build、Python153+2skip，以及Windows六crate/壳all-targets交叉check均通过；独立审查无剩余阻断。见[联合回归](docs/verification/2026-10-04-runtime-policy-and-final-regression.md)。新版Linux release与真实固定Qwen0.6B harness通过，含重复短测、停止后离线证明与取消；Windows external链路仍未运行。随后clean提交、重新捕获来源并构建一个Windows包及一个新PR；main仍为26206ef且未由开发方更改，目标Windows待用户实机验收。

## 2026-10-04 Windows 本机测试记录修复（源码验证完成）

本批冲突已在长期开发分支 `codex/nexa-add-model` 通过真实merge提交 `f496aac6da0f28980ceee15211ff4a8eddf0ff26` 解决并推送，父为Add206d965与main26206ef；main未由开发方改动。联合437 Rust/291前端/Python153+2skip及独立交叉回归通过，无残留文本冲突。

当前在同一分支按[ADR0022](docs/decisions/0022-windows-local-validation-paths-and-feedback.md)修复Windows canonical数据根被外部路径规则误拒、本机证明错误被隐藏、本次测试结果/按钮与历史矩阵混用。原始路径不重写，外部UNC/设备/reparse限制不放宽。源码全workspace/all-targets449通过/0失败/7既有忽略（另4项doc-tests通过）、完整clippy/fmt、前端343项/typecheck/lint/build、Python153+2skip通过；Windows四crate all-targets交叉检查通过，真实Windows与本批真实GGUF仍待最终验收。详见[本轮记录](docs/verification/2026-10-04-windows-model-evidence.md)。本检查点未打包。

下一步仍为可配置文件校验超时和不自动卸载，随后统一构建与新PR。用户已明确长期一个开发分支，除确需隔离不再为每功能新开；下文旧“各片独立分支”的历史安排不再适用。

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
- 下一步：形成源码提交并捕获干净源身份，执行新 Windows 交叉构建、真实同源 aria2 重建与完整 ZIP 核验后提供测试包。当前新包尚未完成；不运行 GitHub Actions、不购买资源、不改 vendor/推理 ABI、不恢复其他平台/Harness

## 2026-10-04 云端 Windows 交叉构建（提交前验证快照）

用户因本地构建反复失败，已要求改由云端构建，并明确同意本次 Microsoft Build Tools/SDK 适用条款。当前使用 Linux 云端的独立 Windows x64 MSVC-ABI 测试路径，不使用 GitHub Actions、不购买云资源；用户后续负责运行验收，无需继续自行编译。原生 Windows 两个打包器保持原样，见[交叉测试构建说明](docs/windows-cross-test-build.md)。

- 基线 `ecfa2c21aee52b65957dde9de534ca75704ca7f9`，真实锁定 llama.cpp Git checkout 为 `2149c00f4442dc59302e134a02e4c99d5f7ed9fc`；主仓库使用核对原始 Git object SHA 的显式 shallow sparse checkout，未物化当时完整树时不冒称完整 checkout
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

最后更新：2026-10-03。工程实现、逻辑测试、真实模型、Windows CI、原生窗口、目标设备与后期发行条件分层记录。当前排期见本文件顶部；保留桌面阶段记录，原始版本由Git追溯。

## 当前目标与授权

按 [ADR0014](docs/decisions/0014-windows-desktop-cpu-runtime.md)，Nexa 聚焦 Windows 桌面 CPU 本地 LLM runtime，以 llama.cpp/GGUF 为核心，通过 API 供其他应用调用。Windows10 x64 / i5-8400 / 16GB内存为首要目标，后续按实测扩大 Intel/AMD 桌面 CPU 与 Windows11；桌面 UI 逐步完善为管理器，聊天为辅助。

用户明确要求API兼容官方DeepSeek Harness（dsh）；只读研究已确认rc2基线与pi-ai自定义openai-completions路线，见[harness契约](docs/windows-harness-contract.md)。官方pi-ai的受控文本协议子集已有实测；DSH本体、工具协议/实用模型/真实agent回合未执行或验证，不能把文本子集宣称为完整兼容。用户2026-10-03已明确“可以，现在逐步推进”，后续Windows路线实施已获授权。已完成W00为纯文档；后续W02源码/原生构建与验证独立记录；W04最小文本互通可独立推进，不等待W01目标机窗口或W03托盘。既有开发分支/CI授权不扩张为独立项目、新权限、合并或部署。

本段为2026-10-03的桌面范围记录；后续移动清理以文件顶部和ADR0029为准。独立项目不改。Telegram 摘要为可选参考调用端，不构成 runtime 发布依赖。无开发工具、实际离线和长期稳定性仍列后期验收。

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
- 接口现为严格文本与ADR0032本机单图 Chat Completions 子集，LAN仍仅文本；不包含已验证的工具调用、结构化输出或完整 harness 兼容性
- `status/devices` 未知 native 指标为 null/unavailable；配置值不伪装成实测值
- Windows worker清理未获OS确认时fail-closed，不假称已回收；已有跨层取消/真实故障优先级修复保留
- ASan/UBSan纯流缓冲测试不是全原生库无泄漏证明；长期100请求/20加载趋势仍后期验收
- 当前无托盘、开机自动启动、完整聊天持久化或新硬件加速的实现承诺

## 最小下一步

1. W00已完成；W04窄文本切片已提交35bfd85，其[WindowsCI37087595998](https://github.com/Naza3/Nexa/actions/runs/37087595998)已于02:27 UTC成功，50项证据/身份/hash核验通过；只覆盖35bfd85，不覆盖本次开放模型工作区变更
2. 收口aria2工作树与三补丁Windows源码构建，按同source运行真实源/进程/文件事务/完整包闭环；旧具体被拒分支仍未知，不预称修复。目标机下载→显式扫描/加载完整验收仍未完成
3. W02混合目录43ad5c2已通过WindowsCI、包内验收与独立下载包复核，原字节包已发送；等待用户目标机验收，按[本轮矩阵](docs/verification/2026-10-03-mixed-model-directory.md)逐层记录；旧50c9d41 CI不覆盖该增量。继续其他模型/目标16GB机实测，不扩大已验证矩阵。W04完整DSH/真实模型文本与工具能力缺口保留，按用户要求暂停新实施；pi-ai fixture仍非DSH本体捕获
4. ADR0017的33f0e17已过精确WindowsCI、真实MS固定模型链和独立包/新依赖许可复核并发送；后续记录用户目标机发现、下载→显式扫描/加载与取消分支。HF实际下载与其他7个候选另验。W04新实施暂停，生产工具仍未实现，不恢复其他平台或绑定Telegram业务

## 桌面历史构建

### 开放模型提交与构建状态

8522514的Windows37101303658已cancelled，不记为失败推理或通过。闭包修正50c9d41的Windows37101760025已success，当前产物独立字节闭包复核已通过，原字节包于07:03 UTC发送获接受；目标机下载/运行仍待确认。详见[最终CI记录](docs/verification/2026-10-03-windows-open-models.md#最终50c9d41-windows-ci与交付产物2026-10-03)。
