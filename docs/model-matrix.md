# 模型验证矩阵

日期：2026-10-03。记录Windows精确模型资产与验证范围。小模型链路结果不等于桌面实用质量、工具能力或未测CPU支持；退役平台研究资产由Git历史追溯。

## Windows / Linux 开发 GGUF 基线

本表是验证证据清单，不是允许加载的型号或hash白名单。用户目标为Windows10/i5-8400/16GB、支持广泛模型，按[ADR0015](decisions/0015-open-model-loading-and-validation-evidence.md)分离：

- `validated`：精确模型/模板/引擎/参数/设备的已有历史证据；不可伪造或扩大范围
- `loadable`：受控manifest满足候选条件，可进入加载流程；不是当前文件完整性证明，实际提交native前另须store核验。不要求先列入本表，不承诺成功或质量
- `available`：候选资格与当前观察到的文件/目录状态；真正引擎支持、模板framing和内存仍在load/prepare时判断

开放模型最终50c9d41已通过WindowsCI37101760025及固定Qwen3-0.6B真实回归，桌面包独立字节闭包复核通过，原字节包已于07:03 UTC以Nexa-Windows-x64-50c9d41.zip发送获接受；下载或运行尚未确认；未验用户Win10/i5-8400/16GB及其他模型，旧389eeef仍使用精确模型门槛。文件hash从“许可名单”角色中退出，但继续用于全文件完整性和身份复验。单文件、已实现tensor结构、原始嵌入模板及文本执行边界保持，不能把GGUF扩展名当作成功保证。

Qwen3.5-4B及其他模型未实测不等于被产品名称名单永久禁止；是否适用于锁定引擎、模板/结构和16GB资源，需实际加载与测量。下面唯一已有完整基线继续原样保留，不标成其他模型也通过。

| 字段 | 固定值 |
| --- | --- |
| 标识 | `qwen3-0.6b-q8_0` |
| 来源 | [Qwen/Qwen3-0.6B-GGUF](https://huggingface.co/Qwen/Qwen3-0.6B-GGUF/tree/23749fefcc72300e3a2ad315e1317431b06b590a) |
| revision | `23749fefcc72300e3a2ad315e1317431b06b590a` |
| 文件 | `Qwen3-0.6B-Q8_0.gguf` |
| SHA-256 | `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031` |
| 文件字节数 | 639446688 |
| GGUF / 架构 / 量化 | v3 / qwen3 / Q8_0 (`general.file_type=7`) |
| 原始模板 SHA-256 | `57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361` |
| 许可证 | [Apache-2.0](https://huggingface.co/Qwen/Qwen3-0.6B-GGUF/blob/23749fefcc72300e3a2ad315e1317431b06b590a/LICENSE)，据同 revision 官方模型卡 |
| 本轮 context | 2048；开发 smoke 输出预算按用例限制 |
| 模板模式 | common/chat，Jinja，enable_thinking=false；不向用户文本添加控制词 |
| llama commit | `2149c00f4442dc59302e134a02e4c99d5f7ed9fc` |
| Windows x64 CPU | 已验证T00/T01原生、T02存储/调度及T03独立worker/Job隔离：Server2022 / EPYC7763 CI、2逻辑CPU / 2推理线程、context2048；[T03运行](https://github.com/Naza3/Nexa/actions/runs/36801681068)。真实信用取消与无native父端链通过；4线程超配探针仍60秒超时，不在通过配置内 |

### 50c9d41固定模型回归增量

[WindowsCI37101760025](https://github.com/Naza3/Nexa/actions/runs/37101760025)于2026-10-03 06:50 UTC成功，源码`50c9d41e5de06632b4cbb23de699bd253fec15ac`、tree`04dccaf6cc8ba63611bb3abcc162dd26e057d1d1`。固定模型hash`9465e63a…b031`和**真实GGUF嵌入模板**完整SHA256`57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361`实际核对后，原生/真实模型、store/core/worker、HTTP/CLI与解压desktop bridge回归通过，CPU 2线程/context2048/batch128。

这次才补齐开放实现对该真实GGUF模板与推理的Windows回归，不能由此前SHA`87a2728c…396b5`的纯Jinja fixture等价测试代替。主代理已核50份证据/source/hash，详情见[最终记录](verification/2026-10-03-windows-open-models.md#最终50c9d41-windows-ci与交付产物2026-10-03)。`native_window_tested=false`；其他模型/量化/模板、Windows10原生窗口、i5-8400/16GB、离线与长期条件未因此通过。

文件和模板 hash 已对实际下载文件计算；详细本轮 Linux 开发结果见对应验证记录。模型文件不提交源码仓库，不自动下载到最终产品。


## 内置可下载候选目录（ADR0017，实施中）

[固定源码目录](../crates/desktop-bridge/src/model-catalog.json)含8个单文件GGUF，各源独立锁完整revision、URL、size及SHA256。来源和字节身份用于安全下载，不是允许加载的白名单；其他合法本地GGUF仍可尝试。启动和打开目录不联网，明确下载才访问已保存的MS/HF源。

| 文件 | 精确bytes | 发布来源 | 当前能力证据 |
| --- | --- | --- | --- |
| Qwen3-0.6B-Q8_0.gguf | 639,446,688 | Qwen官方 | 原固定文件Windows CI真实回归；本下载器尚未实测 |
| Qwen3-1.7B-Q8_0.gguf | 1,834,426,016 | Qwen官方 | 未实测候选；加载/文本/工具均需另验 |
| Qwen3-4B-Q4_K_M.gguf | 2,497,280,256 | Qwen官方 | 未实测候选；加载/文本/工具均需另验 |
| Qwen3-4B-Q8_0.gguf | 4,280,404,704 | Qwen官方 | 未实测候选；加载/文本/工具均需另验 |
| Qwen3-8B-Q4_K_M.gguf | 5,027,783,488 | Qwen官方 | 未实测候选；加载/文本/工具均需另验 |
| Qwen3-8B-Q8_0.gguf | 8,709,518,112 | Qwen官方 | 未实测候选；加载/文本/工具均需另验 |
| Qwen3.5-4B-Q4_K_M.gguf | 2,740,937,888 | Unsloth社区量化 | 未实测候选；加载/文本/工具均需另验 |
| Qwen3.5-4B-Q8_0.gguf | 4,482,403,488 | Unsloth社区量化 | 未实测候选；加载/文本/工具均需另验 |

所有条目服务元信息标注Apache-2.0，各自固定来源/许可信息仍须保留。双服务声明size/hash相等、MS HEAD成功与HF重定向仅是研究元信息，不是本下载器全文件验证；HF仓库模板元信息也不是逐个量化文件完整解析。仅0.6B Q8_0有历史精确验证，下载完成不升级validated。

Qwen3.5条目为文本候选，即使锁定llama源码含qwen35也不承诺实际loader/模板可用、视觉或工具支持。8B Q8_0约8.71GB权重，16GB总内存还须供系统/KV/缓冲，未测CPU延迟；其他量化也不能由文件名推断全部tensor布局。context_hint2048是保守试验提示，不是已经配置或可运行容量证明。

本轮保存结果明确saved=true/registered=false，须另扫描。实际传输、Windows保护与产品验收见[新记录](verification/2026-10-03-model-catalog-download.md)；旧43ad5c2没有自动发现或下载功能。

## 开放候选与基准样本（固定旧模型已回归，其他模型/目标机待验）

产品允许范围按实际结构/引擎/模板能力决定，不逐个批准模型名或hash。基准样本可以从0.6B、1.7B、4B等容量档选取，目的为测16GB桌面的内存、延迟和质量，不是把产品限定在这些档位/型号；更大或不同系列的合法候选不因未列入表而自动禁用。

每条已实测记录仍应精确保存以下条件：

| 证据字段 | 用途/限制 |
| --- | --- |
| 来源/许可证/revision/完整hash | 运行输入复现和完整性，不是hash许可名单 |
| llama commit/GGUF架构/张量类型/量化/原始模板hash | 表明到底测试了什么；实际引擎loader/文本契约决定是否接受 |
| CPU/OS/总内存与可用量/加载参数 | 16GB是用户提供总容量，可用量、KV与其他进程影响须实测 |
| context/输出与实际模板token预算 | metadata与131072上限不是可运行内存承诺，不能静默截断 |
| 文本质量与模板边界 | 中英/多轮/system保留，不fallback、不改写角色，不静默丢控制token |
| 工具能力（独立证据） | 模型真实工具请求/参数/无害回合与异常结果；开放文本加载不授予工具标签 |
| 性能/资源 | 冷载、TTFT、prefill/decode、parent/worker内存、取消与卸载；预热后统计，不推定固定速度 |
| 运行/故障 | 不支持结构/分片/模板、资源不足、完整性变化、取消/worker恢复分别记录 |

新模型可先作为“未实测，可尝试加载”候选；运行失败返回具体原因，不自动升级validated，也不通过换模板/换模型假装成功。需要新llama版本时另立升级验证，不自动跟随上游。

managed导入前、manifest与load统一单文件≤16GiB；external原16GiB限额保持。该文件读取/登记预算不是16GB RAM成功保证。已发送50c9d41在metadata context小于默认2048时扫描登记仍失败；ADR0016已由43ad5c2 WindowsCI及包发送验证，仅自动扫描default_context改取min(2048,metadata)，用户显式import/load/UI参数不静默夹紧，真实短context模型仍未验。

当前结构子集包括GGUF v2/v3及已实现常规/K tensor布局；未知layout、分片、无嵌入模板或非受支持执行方式明确拒绝。已发送50c9d41仍可能因单个不兼容文件整批登记失败；本轮[ADR0016](decisions/0016-mixed-model-directory-diagnostics.md)在完整安全扫描后一次发布合法集合，partial带完整有界诊断、全坏保旧、空目录可空提交。所有预算/IO/身份/路径/reparse/取消/timeout/save仍硬失败；实现及43ad5c2 WindowsCI/包发送已完成，用户目标机待验，见[新记录](verification/2026-10-03-mixed-model-directory.md)。候选扫描改进不新增任何模型实测标签。[W02记录](verification/2026-10-03-windows-open-models.md)单列逻辑、模板fixture、真实模型和WindowsCI，不混用通过结论。

本矩阵只维护当前桌面证据；退役平台研究不作为Windows模型准入或发行依赖。


## GLM-OCR 单图 CPU 开发证据（2026-10-07）

按[ADR0032](decisions/0032-local-single-image-ocr.md)增加双文件视觉模型；本节是 Linux 开发证据，不扩大上面的 Windows 文本验证矩阵，不授予 OCR `validated` / `Passed` 标签。

| 项目 | 实际输入 |
| --- | --- |
| 来源 | [ggml-org/GLM-OCR-GGUF](https://huggingface.co/ggml-org/GLM-OCR-GGUF/tree/65a42de1148dbed2297e922b5dbc7d9b70c36578) |
| revision | `65a42de1148dbed2297e922b5dbc7d9b70c36578` |
| 主 GGUF | `GLM-OCR-Q8_0.gguf`，950433408 bytes，SHA256 `45bc244a6446aff850521dc41f18bc8d7105ad5f0c2c8c28af04e7cc4f4d50b1` |
| projector | `mmproj-GLM-OCR-Q8_0.gguf`，484403648 bytes，SHA256 `9c4b58e33e316ed142eb5dcb41abec3844d3e6e5dc361ffb782c3fa9d175141f` |
| metadata | 主`glm4`；projector `clip` / `glm4v`，projection 1536；原始模板SHA256 `4a2644d74dd6c07c2990c5b289a56443982ee368d11fc4645e914e820eb0b4ba` |
| 模型许可证 | 两个实际GGUF的general.license均MIT；不与SDK代码Apache-2.0混淆。GGUF发布卡未注明转换用原checkpoint的精确revision |
| 引擎/设备 | 原锁定llama提交；Linux / Xeon Platinum8573C，4核CPU配额，32GiB内存配额；无GPU |
| 实际HTTP | context8192/batch256/threads4/temp0，输出上限256；合成960×300图片3行全匹配，385输入token，已加载模型单次请求约17.8秒 |
| 行为验证 | 真实双hash导入与关闭重开、native坏图/预算/取消/同模型恢复、HTTP SSE/取消/非流式恢复；普通Qwen文本回归另跑通过 |

固定上游CLI对官方复杂`code.png`只命中4/5内容锚点，遗漏页眉并有XML错误；这条失败观察保留。小合成图通过不等价于表格/复杂版面准确率，也不等价于ocr.z.ai完整流水线。用户Windows10/i5-8400/16GB、其他量化/模板/模型及Windows原生包均另验。完整命令、分层结果和限制见[本轮验证](verification/2026-10-07-local-ocr.md)。
