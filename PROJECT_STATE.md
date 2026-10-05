# Nexa 当前状态

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
