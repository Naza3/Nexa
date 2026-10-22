# 固定 PI serializer 合成请求

由 `examples/pi-desktop-tools/verify.mjs` 调用官方 pi-ai1.0.1 + PI Desktop v0.17.0 patch，并由临时 HTTP 服务实际接收。生成器逐字段复验全部 JSON；原客户端/源码/hash/许可证见该目录的来源锁与说明。

ordinary 为普通文本；core-tools 使用官方参数表达式与明确合成描述；tool-first、tool-second 是无害内存工具的两轮请求；truncated 是第一轮请求配合故意截断的响应。最后一个请求与 tool-first 相同是预期行为。

这些是协议测试，不是实际 PI 原生 GUI、用户聊天或真实模型输出。合成响应的工具参数不能被称为模型生成。
