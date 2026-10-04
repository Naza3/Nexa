# 固定验证输入

- `baseline.json`：T00/T01 开发验证使用的精确上游源码、模型 revision、GGUF SHA-256、原始模板 SHA-256 和参数。模型 hash 由实际文件计算；这里不包含 GGUF 权重。
- `english.txt` / `chinese.txt`：固定中英文短问句。
- `upstream-prompt-zh.txt`：从锁定 GGUF 原模板以 `enable_thinking=false` 渲染的固定中文输入，专用于上游 CLI `-no-cnv -f` 对照；不是在运行时给用户输入拼接控制词。
- `summary-long.txt`：完全合成的长会议记录，用于输入与生成通路验证，不用于宣称摘要质量已验收。

模板 hash 对 GGUF `tokenizer.chat_template` 原始 UTF-8 字节计算，不规范化空白。所有输入文件的 SHA-256 会写入每次 xtask 报告。报告默认不记录提示词、生成正文或完整模型路径。

这些文本与 metadata 文件属于可公开测试数据；不得用真实用户聊天替换并提交。
