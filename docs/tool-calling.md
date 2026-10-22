# 工具调用兼容

Nexa 在 `/v1/chat/completions` 使用 OpenAI 函数工具协议。实现与验证状态见[ADR0044](decisions/0044-openai-tool-calling-compatibility.md)及项目当前状态。工具由客户端执行，Nexa 不获得文件、命令或网络执行权限。

## 请求与响应

支持 `tools:[{type:"function",function:{name,description,parameters}}]`、`tool_choice` 的 `none/auto/required` 或指定函数对象、`parallel_tool_calls`。参数 schema 原样作为有界模板元数据，支持嵌套对象/数组、anyOf/const等；`strict:true` 不支持，普通schema不保证模型生成满足全部schema语义，客户端执行前仍须验证。

assistant工具消息使用 `tool_calls:[{id,type:"function",function:{name,arguments}}]`，arguments为完整JSON对象的字符串；无文本时content为null。客户端把结果作为 `role:"tool",tool_call_id,content` 发送下一轮。允许多个调用及结果逆序，但不能有孤立/重复/缺失结果，也不能复用历史调用ID。

流式输出工具首块给index/id/type/function.name，后续给对应index的arguments增量；正常工具终态是 `finish_reason:"tool_calls"`，随后可选usage和 `[DONE]`。客户端须等待成功结束再执行；出现error、EOF或截断不能因为曾见toolcall_end就执行。

非流式返回同一assistant结构。普通文本请求保持原实时输出；工具相关请求为完整校验后分片，因此首个内容可能更晚。

## 自动适配与限制

无需选择“PI专用模式”。协议层通用，内部从原模型模板和锁定llama.cpp解析能力适配；不按模型名称/hash硬白名单授予工具能力。不同量化仓库可能带不同模板，所以用户文件与测试样例要分别确认。

有工具接口不等于所有模型都会可靠调用工具。模板无法可靠编码/区分工具与文本时返回 `unsupported_tool_calling`；不偷偷注入另一套模型协议、替换模型或忽略tools。无效调用、参数不完整和输出超限各有明确错误，不自动重放。

当前主要限额：64个工具、16个本轮调用、单参数JSON字符串16KiB、工具相关生成原始/正规化输出各64KiB；请求总量、JSON深度和节点、模板token预算另有上限。非流式编码后响应有既有96KiB上限，超限明确失败。schema和工具结果也受独立字节限制，不静默截断。

支持合法 `store:false`，不会据此增加服务端业务历史保存；其他未知或不支持参数仍报错。工具模式不与OCR图像输入混合，工具stop参数按当前明确规则处理。

## 复现

[PI客户端验证目录](../examples/pi-desktop-tools/)保存官方固定来源、patch、请求fixtures、成功终态执行门及真实Nexa两轮测试驱动。它不运行PI桌面GUI；真实模型测试仅执行固定内存查表，不读用户文件或调用外部服务。
