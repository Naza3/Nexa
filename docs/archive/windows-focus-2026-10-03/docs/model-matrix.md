> 历史快照：2026-10-03 Windows 范围收敛前的文档，仅供追溯，不是当前要求或待执行任务。原文事实未重新验收；只增加本说明并修正相对链接。当前入口见 [PROJECT_STATE.md](../../../../PROJECT_STATE.md)。

# 模型验证矩阵

日期：2026-10-02。该表记录精确资产与验证范围，不将小模型的链路结果当作 Telegram 摘要质量或未测试平台支持。Windows GGUF与Android MNN包分开，不能互相继承hash或validated声明。

## Windows / Linux 开发 GGUF 基线

GGUF是文件容器格式，不是运行兼容性承诺。当前Windows只准入下表精确文件、模板、架构与引擎组合；相同系列但不同量化或hash仍未准入。目录登记成功仅说明已取得受控元数据，不表示任意GGUF可加载；列表也不替代实际load时的完整性重验。界面/API按[兼容性契约](../../../t06-model-directory-contract.md#模型兼容性说明windows)解释架构不支持、量化/模板/上下文/精确资产未验证等原因。

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
| Android | 不作为当前MNN资产；GGUF的Windows通过结果不能授予Android支持 |

文件和模板 hash 已对实际下载文件计算；详细本轮 Linux 开发结果见对应验证记录。模型文件不提交源码仓库，不自动下载到最终产品。

## Android MNN 候选矩阵（Linux探针已验证，Android运行未验证）

采用[ADR0008](../../../decisions/0008-android-mnn-engine-and-package.md)；当前是研究输入锁，不是生产schema或支持承诺。

| 字段 | 固定值/边界 |
| --- | --- |
| 来源 | [taobao-mnn/Qwen3-0.6B-MNN](https://huggingface.co/taobao-mnn/Qwen3-0.6B-MNN/tree/34dfccda1187ded6e07ea06426da576b0b793c6b) |
| revision | `34dfccda1187ded6e07ea06426da576b0b793c6b` |
| 文件 | config.json、llm_config.json、llm.mnn、llm.mnn.weight、tokenizer.txt |
| 大小与SHA256 | [候选输入锁](../../../../scripts/android_mnn/candidate-model.json)，逐文件对实际字节核对 |
| 模板SHA256 | `87a2728cb8dc9fe424d624542f6060ec05a1d285ebbec578bb078900e33396b5` |
| MNN | `d407447ed56c4121a11ccbd266dc184ca1ead0c2`，无补丁 |
| 配置 | CPU / precision high / 2线程 / load-time greedy / enable_thinking=false；功能输出预算16，边界预算8 |
| 来源路径与限制 | [ADR0010](../../../decisions/0010-model-artifact-and-conversion-provenance.md)公开预转换路径；exporter commit/原始Qwen revision未知，不宣称转换可复现，不因这两项未知单独否决运行资产准入；发布者/模型许可/发行闭包仍须复核 |
| Linux x86_64 | 4类真实合成输入、重复一致、36/35预算边界通过；详见[T07-A](../../../verification/2026-10-02-t07a-mnn-cpu-probe.md) |
| Android arm64 CPU | 原生CLI交叉构建/16KiB LOAD对齐通过；模型加载/生成/真机性能与生命周期未验 |

| 后端 | 资产与验收待办 | 当前状态 |
| --- | --- | --- |
| CPU生产资产 | 来源/许可、包schema/引用闭包、生产Executor与设备准入；运行输入锁与转换来源分别记录 | 未完成；研究候选不能称正式支持 |
| OpenCL | 兼容变体、CPU对照、驱动/profile/fallback、生命周期 | 未开始 |
| QNN v79/v81 | 各SoC图、量化/校准、SDK/runtime/图hash及兼容CPU资产 | 未开始 |
| 直接Hexagon | 独立W4对称/C4候选、Host/DSP库、v79/v81分别验收 | 未开始；不能继承QNN结果 |

公开参考设备档和官方来源见[计划第3节](../../../t07-android-mnn-plan.md#3-版本研究基线与设备矩阵)。ABI/页大小/内存/驱动与支持范围必须由实际诊断确认；文档中的型号或上游模型条目不构成设备通过证据。
