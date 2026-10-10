# ADR0044：OpenAI 工具调用兼容层

- 日期：2026-10-10
- 状态：已实现，Linux真实模型闭环验证完成；Windows分层待验
- 范围：用户在 PI Desktop 调用 Nexa 遇到 `unsupported_parameter: tools` 后明确要求补齐功能，并强调应为通用兼容模式。目标模型口述为 Qwen3.5-4B-Q4，完整用户文件身份尚未确认。

## 决定

在现有 `/v1/chat/completions` 中实现标准函数工具协议，不增加 PI 专用推理端点或按模型名称放行的名单。HTTP、actor、worker IPC、原始模板与原生解析共用明确类型。Nexa 不执行工具；读文件、命令、网络等授权与执行均属于调用客户端。

1. 工具定义保留完整有界 JSON Schema 元数据，支持 `none/auto/required/指定函数`，支持多调用及 `parallel_tool_calls:false` 的单调用限制。`strict:true` 明确拒绝；不把普通参数描述说成约束解码或全 JSON Schema 校验。
2. 工具历史保留 nullable assistant content、tool_calls 与 tool_call_id；结果可以不同顺序返回，但必须匹配唯一、未重复、完整的调用 ID。当前定义移除历史工具不使有效旧历史失效。
3. 工具相关请求（定义或工具历史存在）先有界收集并严格校验完整生成结果，再输出标准 text/tool delta。SSE 是校验后分片，不是实时参数解码；支持同语义的非流式响应。普通无工具文本继续原有实时流。
4. 模型原始模板及锁定 llama.cpp 共用模板/自动解析器负责适配。严格 PEG 必须全量匹配，使用原始 AST 参数片段或上游参数类型标签映射，禁止 lenient JSON 补全、手写型号标签修补或把任意 JSON 正文当调用。模板不具备安全工具协议能力时明确报错，文本加载资格不受模型名称/hash名单限制。
5. 真实模板展开后的完整 prompt（含工具/schema/历史/result/prefix/特殊token）重新计算 token 预算；不丢弃历史或参数来规避上下文上限。
6. 原生输出在整段校验、Rust 名称/选择/JSON/ID校验全部通过后发布；长度截断、无效输出、超限或取消不能产生成功 tool_calls 终态。客户端必须等待成功终态后执行工具，不得仅凭 toolcall_end 事件执行。
7. `store:false`、空 tools 的合法无操作形式纳入兼容；其他不支持参数仍明确报错。不能通过静默忽略非空工具定义来声称兼容。

## 有界输出与生命周期

原文本输出账本保持256KiB。工具请求另设768KiB同一affine账本，保留384KiB用于原生raw/normalized、Rust适配层、IPC/actor参数校验等并存的有界输出副本；每份数据主体上限64KiB，剩余空间覆盖ID/名称与容器元数据。字符串必须精确扩容/收缩并验证实际capacity，不能以len代替长期保留容量。

信用与普通文本统一：最多2份120KiB transit、16KiB scratch、96KiB非流式最终编码缓冲，加384KiB保留共736KiB。工具header和arguments都需要同一类一次性信用，不产生免费事件通道。取消、输出队列满、worker崩溃和关闭仍保证一次终态；携带保留permit的终态不能因取消而被当普通payload丢弃。保留permit转移至actor终态envelope，直到处理并销毁校验副本。

这个账本约束待消费输出，不是整个模型进程RSS上限。上游模板、token、PEG/JSON构建期临时工作区与模型内存单列；输入/深度/节点/输出有界，但不能宣称768KiB覆盖所有原生瞬态内存。资源错误仍按安全失败处理。

## 版本与兼容

公共API版本不变；私有worker协议与shim行为身份从4原子迁移到5，旧worker明确拒绝。C ABI添加v3工具入口，旧v1/v2布局和文本/OCR入口保持；API管理进程仍不链接原生库。打包清单、验收器及测试同时检查新身份。

`docs/windows-tools-contract.md` 的旧8工具/标量schema/仅流式/单调用/IPC2方案被本ADR及当前说明取代，不再作为实现门槛。T0探针保留为上游宽松解析危险的历史证据，不转授为模型实测。

## 验证分层

- 类型、DTO、完整JSON/重复键、ID状态机、工具选择及超限负例。
- IPC实际编码/读回、信用、乱序、错误终态、队列满取消、预算capacity。
- 真实HTTP + 合成executor的流式/非流式、当前模型原子绑定、工具结果回传。
- PI Desktop v0.17.0依赖的pi-ai1.0.1 + 官方patch实际serializer/parser夹具；独立于旧0.87.1 Harness回归。
- 锁定真实GGUF两轮无害内存工具闭环，以及普通文本/OCR回归；具体模型/模板/hash和结果分别记录。
- Windows交叉、原生CI、用户Win10和PI Electron GUI分别验收。Linux通过不是这些层的替代品。
