# 模型验证矩阵

日期：2026-10-02。该表记录精确资产与验证范围，不将小模型的链路结果当作 Telegram 摘要质量或未测试平台支持。Windows GGUF与Android MNN包分开，不能互相继承hash或validated声明。

## Windows / Linux 开发 GGUF 基线

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

## Android MNN 候选矩阵（全部未实现/未验证）

采用[ADR0008](decisions/0008-android-mnn-engine-and-package.md)方向，先研究公开Qwen3-0.6B文本小模型；尚未选择/导出/校验具体MNN包，不把同名GGUF、上游发布样例或候选名称视为支持。

| 路径 | 资产与验收待办 | 当前状态 |
| --- | --- | --- |
| CPU | 原模型revision/许可、MNN导出器commit/参数、graph/weights/tokenizer/config/template闭包、整体hash与真实CPU基线 | 未锁定、未构建、未测 |
| OpenCL | 明确兼容变体与CPU对照、驱动/profile/真实fallback、生命周期 | 未开始 |
| QNN v79/v81 | 各SoC目标图、量化/激活/校准、SDK/runtime/图hash与兼容CPU资产分别记录 | 未开始 |
| 直接Hexagon | 独立W4对称/C4候选变体、Host/DSP库与v79/v81目标分别验收，不能复用QNN支持结论 | 未开始 |

公开首测参考OnePlus 15 / SM8850 / v81；SM8750 / v79为兼容档，官方来源见[计划第3节](t07-android-mnn-plan.md#3-版本研究基线与设备矩阵)。Android版本、ABI/页大小、内存、驱动由实际诊断确定。详细准入须绑定包/模板/配置身份、实际后端和设备报告；新包schema在T07-B冻结前不发布虚构的manifest示例。
