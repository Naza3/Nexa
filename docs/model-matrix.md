# 模型验证矩阵

日期：2026-10-03。记录Windows精确模型资产与验证范围。小模型链路结果不等于桌面实用质量、工具能力或未测CPU支持；旧移动研究资产见历史快照。

## Windows / Linux 开发 GGUF 基线

GGUF是文件容器格式，不是运行兼容性承诺。当前Windows只准入下表精确文件、模板、架构与引擎组合；相同系列但不同量化或hash仍未准入。目录登记成功仅说明已取得受控元数据，不表示任意GGUF可加载；列表也不替代实际load时的完整性重验。界面/API按[兼容性契约](t06-model-directory-contract.md#模型兼容性说明windows)解释架构不支持、量化/模板/上下文/精确资产未验证等原因。

Qwen3.5-4B仍未列入运行矩阵；不能用文件名、量化后缀或当前llama上游宣传替代本项目锁定引擎的真实验收。

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

文件和模板 hash 已对实际下载文件计算；详细本轮 Linux 开发结果见对应验证记录。模型文件不提交源码仓库，不自动下载到最终产品。


## 桌面实用模型与harness准入（W02规划）

现有0.6B用于链路验证，尚无已通过的4B实用模型或可靠工具模型。下一轮按CPU可用内存、文本/代码/工具质量和响应时间分档选候选，不把型号或参数量视为已支持。4B只是候选容量档，具体模型与量化在锁定资产与llama源码核对后选定。

| 每个候选必须记录 | 准入门槛 |
| --- | --- |
| 来源/许可证/revision/完整SHA256 | 真实文件与许可核验；不同量化/hash分别准入 |
| llama commit/GGUF架构/量化/模板hash | 固定引擎实际支持；不采用最新上游宣传替代当前源码 |
| CPU/OS/可用内存/加载参数 | Windows10/i5-8400优先，实际内存和最佳线程未知须实测 |
| context/输出/tools+history token预算 | 模板后准确计数，不静默截断或放大声明窗口 |
| 文本能力 | 中英文/多轮/质量样本、非思考模式与拒绝边界 |
| 工具能力（独立标签） | 无害工具回合、tool名称/参数JSON、异常输出、取消与多轮；文本通过不授予工具通过 |
| 可选思考能力 | 仅实际目标需要时独立字段/预算/模板验收，当前没有保证 |
| 性能与资源 | 冷载、TTFT、prefill/decode、parent/worker内存、空闲/卸载、取消；至少预热后5次统计 |
| 完整运行 | load/卸载/错误/目录完整性/worker与HTTP真实回归；支持范围精确到组合 |

初期每次只引入一个有明确用途的候选。需要升级llama才能支持新架构/模板时，单独记录升级并回归原有准入模型，不以UI放行或改hash名单代替真实验证。

最新389eeef仅增加兼容原因可见性，没有扩充本表模型支持。固定模型在[最终CI](verification/2026-10-02-windows-model-compatibility.md)继续真实回归；原生窗口与目标CPU性能仍分开记录。

旧Android/MNN输入锁与研究后端矩阵完整保留于[原矩阵快照](archive/windows-focus-2026-10-03/docs/model-matrix.md)，不构成Windows支持或发行条件。
