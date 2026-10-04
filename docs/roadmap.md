# Nexa Windows 后续开发路线

日期：2026-10-03。范围由 [ADR0014](decisions/0014-windows-desktop-cpu-runtime.md) 确定；动态进度只在 [PROJECT_STATE.md](../PROJECT_STATE.md) 维护。本路线是计划，不代表新功能已经实现或新硬件已通过验收。

## 1. 产品与依赖

Windows 10 x64 / i5-8400 / 16GB内存优先，以固定 llama.cpp/GGUF 提供本机 API；后续按实测扩展 Intel/AMD 桌面 CPU 与 Windows 11。桌面管理器服务于 runtime，聊天为辅助。Android、GPU/NPU、Telegram 业务均不作为本轮依赖。

T00–T05 已有阶段证据；T06 大部分能力已实现，当前缺口见 W01。W 编号只表示本次 Windows 后续切片，不重写历史 T/A 编号。

```text
W00 范围与文档 ─┬→ W01 当前版本短验 → W02 开放模型/CPU性能
               └→ W04 文本协议/联通（与W01并行，不等待托盘）
W02 工具模型能力验证 + W04 文本/工具协议 → W04 真实工具闭环
W01 → W03 桌面管理器；W04 文本优先于非必要托盘美化
W01 + W02 + W03 + W04 → W05 后期发行验收
```

W04的官方dsh rc2/pi-ai协议研究已完成，可与W01并行建立客户端fixture/文本联通；真实工具闭环还依赖W02模型工具能力证据。W04协议/文本联通可与W02模型准入并行，优先于非必要托盘美化，不被W03阻塞；真实工具闭环须有W02工具模型证据。W02与W03/W04各自处理独立代码时可以并行，不共享写入同一文件。等待目标机短验时可推进独立研究与自动化，不把等待写成通过。

## 2. 阶段交付与验收

| 阶段 | 已有基础 | 本次最小增量 | 阶段验收 |
| --- | --- | --- | --- |
| W00 主线收敛 | 已有 Windows runtime、历史混合平台文档 | 当前文档去移动依赖，归档历史，冻结 CPU/API 产品边界 | 入口/架构/规格/路线一致；历史可追溯；源码、工作流、Android WIP 未改 |
| W01 当前版本短验 | 已发送33f0e17包、自动发现/双源下载、混合目录/诊断、零复制/自动名、聊天/取消、HTTP与CLI已实现 | 收口新包原生 UI 和一个独立 API 调用方最短闭环，修复有证据的问题 | Windows10 上选择目录→识别名称/准入→加载→生成/停止→关闭窗口后另一客户端仍可调用；剪贴板与错误分支逐项记实 |
| W02 开放模型与 CPU 性能 | context/threads/batch/输出预算、空闲卸载控件已有；状态有部分指标 | 开放多模型候选加载、历史validated与loadable分离、16GB桌面基准样本/工具能力验证、资源观测与参数推荐；明确 CPU 指令集范围 | 固定模型/参数/版本；上游与 Nexa 对照；冷加载、TTFT、prefill/decode、内存、取消、空闲成本；Win10/i5-8400 首测，其他 Intel/AMD 各有证据 |
| W03 桌面管理器 | 模型/聊天/设置页、启动/停止、两种关闭行为已有 | 托盘状态/重新打开、清晰区分关窗口与停服务、API 接入状态/诊断入口；复用现有服务控制 | 原生窗口/托盘反复打开关闭；运行中退出提示不误停其他客户端；服务故障/恢复、单实例和重连无重复服务；harness 所需设置随 W04 契约接入 |
| W04 API 与 deepseek harness | 严格文本 Chat Completions 子集、SSE、鉴权、模型列表/管理、取消已有 | 基于已确认dsh rc2/pi-ai契约冻结实际lockfile/出站fixture，补工具协议和可复现接入示例 | 精确客户端版本/配置连本机 Nexa；请求/流式/错误/取消与必要工具回合逐项通过；不支持项明确报错；无云 API 冒充本地通过 |
| W05 后期发行验收 | 便携包/桌面包/独立工具、CI/包完整性证据已有 | 无开发工具、实际离线、长期稳定性、更新/回退与数据保留、Windows11支持矩阵；公开发行准备另行决策 | A19/A20 等实际环境门槛、W 阶段汇总、全新/升级/故障场景与依赖许可；每个声明支持的组合均有报告 |

用户2026-10-03最新优先事项为完成模型目录发现和下载后加载。按[ADR0017](decisions/0017-model-discovery-and-catalog-download.md)先收口EXE/models未配置发现、固定双源目录、默认MS设置、显式保存→扫描→加载；33f0e17已通过精确WindowsCI、默认MS固定0.6B真实下载及后续推理/完整包/bridge；独立下载包复核通过，原字节包已于13:42:49 UTC发送获接受；用户下载/运行未确认，目标机原生窗口、HF实际传输及其他7个候选仍待验。W04新Harness实施暂缓；其旧文本/T0证据保留，不混入本轮交付。

## 3. W01 最小下一步

1. 使用已发送 `Nexa-Windows-x64-33f0e17.zip`，退出旧 UI 和服务后解压到新目录，记录 OS/CPU、包 hash 和模型 hash；下载/运行尚待用户确认
2. 无配置时检查EXE/models自动发现；已有配置路径优先。服务停止时选择模型目录，先按默认MS下载，再显式扫描、启动/加载；也可选择已有合法GGUF。确认零复制、自动名称与已验证/未实测提示，在包外检查混合partial、全坏保旧、空目录清空及取消/重扫；原文件不被覆盖
3. 真实聊天、停止、再请求，检查剪贴板；明确记录未执行分支
4. 用同一个 data-dir 与本机端点完成独立 HTTP 调用；关闭 UI 后 API 仍可服务，同时退出时能确认 worker 与实例清理
5. 将通过、失败、未执行分别记入新报告。没有新包窗口证据时，不能借旧包 UI 或 CI bridge 结果收口

W01 不重复造目录选择、自动命名、API、聊天、线程控件或两种退出行为。若用户暂时不方便做目标机短验，保留W02模型候选审计与验证准备；当前W04新实施按用户要求暂停，不重新询问已明确的harness项目。

## 4. W02 开放模型与16GB CPU边界

- 现有发布构建已关闭隐式 native 优化与 CUDA/Vulkan/Metal；实际 x64 指令集见 [构建锁](build-lock.md)，不能仅凭 `GGML_NATIVE=OFF` 宣称支持所有 x64
- i5-8400 是目标，不预设最佳线程数；按可用核、短/长输入与其他桌面负载比较，记录响应性和吞吐取舍
- 现有API默认context4096/batch512，桌面验证档2048/2线程/128；历史只证明精确测过的组合。开放候选context仍受metadata与131072硬限约束，16GB可用量/内存需实测，不能从硬上限推断成功
- 按[ADR0015](decisions/0015-open-model-loading-and-validation-evidence.md)先移除模型名/hash运行白名单，保留安全检查；0.6B/1.7B/4B等仅基准样本，矩阵记录已测证据。引擎实际load判断架构/张量，未实测候选可尝试，文本成功不等于工具能力通过
- 性能报告记录条件、中位数与范围，不承诺固定 tokens/s、包体或内存；硬件加速不并入 CPU 主线

开放模型50c9d41已由WindowsCI验证固定GGUF并发送，其他模型/用户目标机仍待验，不能授予新模型成功结论。单文件常规tensor子集之外、分片、缺模板或不支持文本输出明确报错；没有新增Job RAM硬限制，不承诺16GB可装下任意模型或完全隔离OOM。

本轮[ADR0016](decisions/0016-mixed-model-directory-diagnostics.md)混合目录切片43ad5c2已通过本机回归/独立审查及[WindowsCI37108375458](https://github.com/Naza3/Nexa/actions/runs/37108375458)（344 pass/0 fail/7 ignored、external17、CTest3/3、固定GGUF/包/bridge）；下载原字节包独立复核通过，43ad5c2于08:45:45 UTC发送获接受，用户下载/运行、原生窗口/目标机仍待验：完整安全扫描后只把确定内容问题作为逐文件拒绝，合法集合一次partial原子发布并显示完整有限诊断；全坏保旧目录/index/generation，无候选可空提交。所有预算（含parser）、I/O、身份/路径/reparse、取消/timeout/save仍硬失败，坏文件计全部预算；诊断不持久化，公共协议/native/library schema不改。scan-only核旧目录身份，apply可显式换目录；旧50c9d41包仍整批失败，不追溯赋能；43ad5c2的partial已获合成事务/Windows guard证据，但未据此宣称多模型质量或目标机性能通过。

managed导入前、manifest与load统一单文件≤16GiB；external原16GiB限额保持。该文件读取/登记预算不是16GB RAM成功保证。50c9d41默认2048扫描对短context的限制仍在；本轮仅自动扫描default_context改取min(2048,metadata)，显式import/load及UI设置保持。新结果见[验证记录](verification/2026-10-03-mixed-model-directory.md)，不能把候选登记当真实加载/性能已验。

### W02 当前优先增量：发现与下载

- 未配置且服务停止时只发现已有EXE/models；已保存missing/stale路径优先，不隐式回退或联网
- 固定8条、每源独立revision/size/hash；本地列表无远程查询，MS默认/HF可选，明确点击才下载，不自动切源
- 下载服务停止快拒、真实字节/校验、取消/有限关闭、自有.part和no-clobber；saved不等于registered，用户再扫描/加载
- 原普通扫描/模型开放策略保持，其他本地GGUF可尝试；新设置回退旧版需备份或移除download_source，旧UI“默认值”不解决兼容
- 33f0e17 WindowsCI已由产品下载器经MS获取固定0.6B，独立hash符合基线并复用到真实推理/包链；TLS依赖许可及下载包闭包独立审查通过。该证据不替代HF实际下载、其他7个候选或Win10原生窗口验收

本轮[验证记录](verification/2026-10-03-model-catalog-download.md)独立于43ad5c2混合目录和4d30bfa T0测试CI。33f0e17新包已通过CI和独立复核，并于13:42:49 UTC原字节发送获接受；用户下载/运行未确认，W01与完整W02仍未收口，旧43ad5c2不追溯获得自动发现/下载能力。

### W02 当前增量：受控aria2下载引擎

用户要求复用通用开源下载能力，已采纳[ADR0018](decisions/0018-generic-download-engine-candidate.md)的aria2 1.37.0 sidecar。工作树已实现来源适配、进程监督、任务内恢复、exit8一次全量重启、数值进度及父端完整性/发布；不再自建HTTP/Range。源构建、三份本地补丁与产品组件身份/对应源码材料独立固定。

本片门槛为最终回归/独立审查、Windows真实sidecar/进程与文件保护、MS/HF实际下载和完整包闭环；新证据未完成前不把原拒绝错误称为已修，也不宣称已交付新引擎。跨App重启恢复/代理留后续，清洁机、离线、长期稳定性维持W05，不追加为本片完成条件。SChannel平台证书联网边界、崩溃残留规则与当前分层结果见[aria2记录](verification/2026-10-03-aria2-download-engine.md)。Harness新实施保持暂停。

## 5. W04 兼容门槛

准确目标是官方deepseek-ai/deepseek-harness（dsh），研究基线rc2 `639ed015`。优先走其pi-ai自定义openai-completions provider，复用Nexa现有`/v1/chat/completions`；默认deepseek-official讲Messages，只改base URL不兼容，不在首期增加第二协议网关。

完整配置草案、固定源码与H01–H12见[harness契约](windows-harness-contract.md)。实施顺序：精确lockfile/配置 → 实际脱敏请求fixture → 无工具真实文本smoke → tools/历史/工具SSE协议 → W02实用模型的无害工具闭环 → 全阶段Stop/错误/预算。W03管理器仅提供所需接入/诊断入口，不复制harness功能。

必须处理普通tools JSON Schema、assistant.tool_calls、role:tool、null assistant及分片tool参数；schema不等于strict约束解码。兼容开关显式关闭store/developer/strict，选max_tokens，usage按已验能力；显式模型真实contextWindow/maxTokens，不继承262144/32768。默认300秒流空闲超时需与CPU冷载/prefill协调，联调先maxRetries=0，不能用重试掩盖失败。

首期不扩tool_choice、response_format、stop或思考能力，除非实际目标需要。工具执行在harness，Nexa不因此执行系统操作。不能放宽localhost/Host/Origin/Bearer或伪造模型工具能力。分别报告文本联通、工具wire和指定模型agent闭环，当前均尚未通过本轮真实联调。

## 6. 后期与非目标

无开发工具、离线和长期稳定性保持后期验收，不阻塞当前开发，也不能标成已通过。Windows11与新增Intel/AMD机型独立记录；GPU/NPU、移动平台、联网公开服务、完整聊天产品与 Telegram 接入需要单独范围，不混入本次路线。

## 7. 状态与证据

状态使用 未开始 / 进行中 / 待验证 / 已完成 / 受阻；依赖未完成通常是未开始，不滥用受阻。报告包含源提交、引擎/模型/模板hash、设备、参数、命令/退出码、pass/fail/skipped/unavailable、已知限制。CI、原生窗口、目标硬件、离线与长期验收分层记录。

旧 T00–T10、S00–S04 和 Android 验收保留于 [历史索引](archive/windows-focus-2026-10-03/INDEX.md)。历史编号不改造成新 Windows 证据。用户已于2026-10-03授权按路线逐步推进。本次W00仍纯文档，不运行大型构建；后续独立切片按授权实施并分别验收。
