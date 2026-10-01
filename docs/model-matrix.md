# 模型验证矩阵

日期：2026-10-01。该表只记录开发链路候选模型，不将小模型的链路结果当作 Telegram 摘要质量或 Windows/Android 真机支持。

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
| Windows x64 CPU | 已验证T00/T01原生链路及T02真实存储/调度：Server 2022 / EPYC CI、2逻辑CPU / 2推理线程、context2048；[最新T02运行](https://github.com/Naza3/Nexa/actions/runs/36796147278)。独立mid-prefill取消有数值证据；4线程超配探针仍超时，不在通过配置内 |
| Android arm64 CPU | unavailable；未构建/未真机运行 |

文件和模板 hash 已对实际下载文件计算；详细本轮 Linux 开发结果见对应验证记录。模型文件不提交源码仓库，不自动下载到最终产品。
