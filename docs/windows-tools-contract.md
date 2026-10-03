# Windows 工具调用纵向切片契约（设计，尚未实现）

日期：2026-10-03；审计源码：`35bfd85ca38d5e3c781c6b387794b265f29df18c`。本文件及其合成 fixture 只定义下一实现，不表示已经开放 tools、通过工具测试或兼容完整 DeepSeek Harness。既有文本基线见 [Harness 契约](windows-harness-contract.md) 和 [文本验证](../examples/harness/README.md)；当前生产行为仍以源码为准。

用户最新要求：目标机内存 **16 GB**，希望支持很多模型，而非限定几个模型。下面的模型 hash 用于复现实验，**不作为未来产品的模型白名单**。旧源码的精确 0.6B 准入是当前事实，开放模型兼容改造另行协调；本文件不暗中修改该门槛或已有主文档。工具生产实现等待开放模型纵向切片及其 Windows CI 收口，本设计可以先冻结审查。

## 1. 首片边界与完成定义

首片目标：固定官方 pi-ai 自定义 OpenAI Completions 路由，完成 `tools + 工具历史 → 真实 worker/shim → 标准工具 SSE → Harness 执行一个合成只读工具 → tool 结果回传 → 模型最终回答`。Nexa 不执行工具，不新增命令、文件、网络权限或通用执行器。

- 首片采用**完成校验后发送**：含 tools 或工具历史的请求先有界保留模型输出，正常终止并校验后，才发文本/工具数据分片。因此此类请求即便最终只答文本，首个有内容事件也会延后；不是实时参数流。HTTP 初始 role 帧、排队/运行状态均不是可执行工具结果
- 无 tools、无工具历史的普通文本请求保留已有实时 SSE、非流式、stop/length 行为
- 首片工具模式只接受 `stream:true`；`stream:false` 显式报不支持。这样不在同一片加入第二份非流式整包聚合及其额外信用证明；固定 DSH 路由实际使用流式，非流式工具另列增量
- 工具模式不接受非空 `stop`、`response_format`、strict grammar、`tool_choice:required`/指定函数、`parallel_tool_calls:true`、思考字段或多模态。缺省/false 的 parallel_tool_calls 表示不请求并行，不授权执行。先固定至多一个本轮生成调用；多 index 的传输设计与拒绝用例仍保留，扩展前不能宣称 H06 多调用分支已过
- `strict:true` 拒绝；`strict:false`/缺省只表示无约束解码。参数小 schema 的完成校验不是 JSON Schema 全规范，也不是 sampler grammar
- 单工具闭环、普通文本、真实 DSH 以及工具质量分别留证。设计参数不是性能实测；合成 fixture 不是 DSH 实际出站记录或真实模型结果

本文件只提出增量契约。实施时必须同步架构/执行规格/Harness 差异、必要 ADR、状态及验证页，不能让新文件单独覆盖旧规范。

## 2. 已核验缺口与上游可复用能力

| 层 | 当前事实 | 本片需要 |
| --- | --- | --- |
| HTTP | `runtime-api/src/dto.rs` 拒绝 tools、null content、tool role；未知字段严格拒绝 | 有界 tools、小 schema、工具消息状态机；保持精确字段拒绝 |
| 内部类型 | `runtime-types` 的 `Message.content:String`，三个 role，严格交替且末尾 user；无工具结束原因 | 自有工具 DTO，保留 null 语义，末尾 tool 合法，`ToolCalls` 完成原因 |
| C ABI | `air_message` 只有 role/content；v2 文本回调 | 添加 v3 工具入口/结构化回调；不改 v1/v2 布局 |
| 模板 | `air_prepare` 只取 applied.prompt，重复文本交替检查 | 传完整 tools/历史；保留 parser/特殊 token/额外 stops 配置 |
| 生成 | token_to_piece 的 special=false；只输出文本 | 模板识别所需特殊 token 保留、完整解析、完成校验与结构化分片 |
| IPC/调度 | 本文审计基线私有 IPC v1、shim2，信用只认 TextDelta | 同一信用账本覆盖工具与保留输出；基于开放模型切片结果重新锁定一个原子版本元组 |
| 安全 | API 父进程无 native；仅 `llama_log_set` 静音 | 继续无 native；先覆盖 common 独立日志及异常正文 |

接口研究开始时本地子模块未物化，故通过远端精确 commit 只读核对；随后已恢复为 `2149c00f4442dc59302e134a02e4c99d5f7ed9fc`、tree `48255d3c5006bdefffcf8d0af5225b6cc5d4247c` 的干净子模块。本文没有执行 native 验证，未用 master 替代：

- [common/chat.h](https://github.com/ggml-org/llama.cpp/blob/2149c00f4442dc59302e134a02e4c99d5f7ed9fc/common/chat.h)：tool 的 name/description/parameters、消息 tool_calls/tool_call_id/tool_name、模板 tools/tool_choice、返回 parser/grammar/preserved_tokens/additional_stops、partial parse/diff/稳定 ID
- [parser 初始化与解析](https://github.com/ggml-org/llama.cpp/blob/2149c00f4442dc59302e134a02e4c99d5f7ed9fc/common/chat.cpp#L1441)：`common_chat_parser_params(applied)` **没有加载 parser**；须显式 `parser_params.parser.load(applied.parser)`。空 parser 可能回退纯文本，不能把它当工具能力成功
- [server 参考流程](https://github.com/ggml-org/llama.cpp/blob/2149c00f4442dc59302e134a02e4c99d5f7ed9fc/tools/server/server-task.cpp#L162)：累积 → partial/final parse → 稳定 ID → compute_diffs。复用 common 即可，不引入 llama-server
- [common/log.h](https://github.com/ggml-org/llama.cpp/blob/2149c00f4442dc59302e134a02e4c99d5f7ed9fc/common/log.h)：独立日志线程；verbosity 与暂停接口不是线程安全。现有 llama 回调不覆盖该路径

### 原生隐私必须先于工具接入

引擎唯一所有线程在首次 common 调用前设置 `common_log_set_verbosity_thold(-1)`、关闭 JSONL，并暂停 `common_log_main()`；保持现有 llama/ggml 静音，生命周期中不再恢复或由环境变量打开正文日志。具体调用顺序要按固定 log.cpp 核对；不向 common 配日志文件。

parser 的 `LOG_WRN` 可能包含完整生成内容；compute_diffs 异常可能含 arguments。所有 C++ 异常映射为固定错误码与不含输入的短消息；不把 `what()`、原始 parser 错误、Debug DTO、工具结果或完整路径写进 HTTP、stderr、证据。回调 panic/异常不可跨 ABI。合成 canary 必须覆盖错误、取消和关闭，分别扫描 stdout、stderr、错误体与落盘报告；正常响应正文只允许发给已认证请求方。

## 3. API 输入契约

### 3.1 tools 与选择策略

`tools` 缺省或空数组表示没有当前可选工具；每项仅允许 `type:"function"` 和 `function:{name,description?,parameters,strict?}`。

- name：ASCII `[A-Za-z_][A-Za-z0-9_-]{0,63}`，区分大小写，本次定义唯一；不是可执行路径
- description：可缺省为空；parameters 必须为下述对象 schema；strict 只可缺省/false
- tool_choice 缺省：有工具时 auto，无工具时 none；显式仅支持 auto/none。auto 但没有工具拒绝。none 仍可携带历史与定义，模型本轮若产生调用即失败，不能静默丢掉调用
- 首片可提供多个候选定义以测试选择，但只允许本轮生成一个调用。超过一个是 `invalid_tool_output`，没有任何成功工具事件，不只保留第一个
- 其他根字段继续按既有严格契约处理。不能因为打开 tools 顺便接受 store、developer、legacy functions/function_call 或未实现的兼容字段

### 3.2 首片小 schema（明确不是完整 JSON Schema）

根对象允许且仅允许：`type:"object"`、`properties`、`required`、`additionalProperties:false`；其中 properties 必需，required 缺省为 `[]`，additionalProperties 必须显式为 false。每个 property 仅允许 `type`、可选 description、可选 enum；type 只能是 string/integer/number/boolean。enum 为非空、同型、无重复标量数组。required 无重复且每项都在 properties 中。

不支持 `$ref/$defs/$schema`、组合/条件、pattern、format、default、嵌套对象/数组/null、数值范围或长度关键字。任何这类关键字均返回 HTTP400 `unsupported_parameter`，param 指向固定字段位置；不能删除关键字继续、不能声称完整 schema 校验。格式错误、重复字段或自相矛盾则 HTTP400 `invalid_request`。在实现前用固定 DSH **实际** serializer/工具定义审计此范围；默认工具集合不在支持范围时明确列出差异，不能把它称为默认 agent 兼容。

完成校验同时检查：arguments 是恰好一个完整 JSON object、无重复 key/尾随内容、无缺失 required/额外属性、类型及 enum 正确。integer 限精确安全整数 `[-(2^53-1),2^53-1]`；number 为有限 JSON 数值，拒绝溢出与 NaN/Infinity；字符串按 UTF-8 字节限额。无需新增通用 schema 依赖，复用锁内 serde_json 并实现此有限规则；共享 JSON 验证不得引入 native/HTTP 依赖到 runtime-types。

### 3.3 messages 与关联状态机

协议原样保留三种 content 状态：缺省、null、字符串；仅 assistant 且 tool_calls 非空时允许缺省/null，内部统一为 `None`；空字符串仍为 `Some("")`。system/user/tool 的 content 必须字符串。assistant 无 tool_calls 必须字符串。content 数组与 reasoning 一律不支持。

assistant.tool_calls 每项必须有唯一 `id`、`type:"function"`、`function:{name,arguments}`；arguments 必须 JSON **字符串**，按上节完整 JSON 规则解析，不能接受已解码对象。输入 ID 为 `[A-Za-z0-9_-]{1,64}`，在整个请求历史中唯一；不能替输入补 ID。tool 消息只含 `role:"tool"`、`tool_call_id`、字符串 content；tool_name 在 shim 内从所引用调用推导，不接受客户端额外 name。

状态顺序：

1. 可选唯一首条 system，然后 user
2. user 后可接普通 assistant，之后必须 user；或接带调用的 assistant，之后进入待结果状态
3. 每个待结果 ID 恰好对应一条连续 tool 消息；拒绝未知/重复/遗漏结果、被 user/assistant 打断、悬空 tool。首片本轮输出一个调用，但输入历史允许每个 assistant 至多四个调用，结果可按任意顺序，通过 ID 关联
4. 所有结果齐备后可接 assistant（包括下一轮调用）或 user；请求末尾可为 user，或已收齐的一组 tool 结果。不能以 assistant 或未完成结果组结尾
5. 历史调用名称必须合法；不必出现在当前工具定义中，因为会话可能已移除旧工具。历史 JSON 必须完整、对象、有界；如当前定义仍包含该名称，再按该小 schema 校验。历史记录不是本轮执行授权；未知当前工具的**新生成调用**必须拒绝

这替代本片路径的简单交替规则，但不放宽孤立 assistant 开头、重复 system 或任意消息顺序。模板拿到完整 tool_calls/ID/结果，不能把 tool 结果伪装 user 或只拼接文字丢失结构。

## 4. 有界资源与完整模板预算

下列是**拟议工程上限，尚未实测**，应由一个共享常量定义与边界 fixture 驱动。bytes 均为 UTF-8 字节，不是字符数；所有加法检查溢出，不自动夹紧或截断。

| 项目 | 拟议首片上限 / 处理 |
| --- | --- |
| HTTP 编码请求 | 保留现有 1 MiB；超限沿用413 |
| messages | 保留128条；工具结果也计一条 |
| 工具定义 | 至多8个；description 每项1024 B；单 schema 8 KiB；定义总编码32 KiB |
| 小 schema | 至多16 properties、每个 enum 至多16项；schema 节点/深度检查在解析前后有界 |
| 历史调用 | 每 assistant 至多4个、历史合计至多32个；每 ID≤64 B |
| arguments | 每调用4096 B；本轮输出一个调用；历史总参数计入请求总额 |
| tool 结果 | 每条16 KiB；所有结果总64 KiB，仍受请求/模板总预算限制 |
| 解码后的输入正文及定义 | 合计≤1 MiB，包括 descriptions/schema/arguments/ID/tool results；不是仅 messages.content |
| IPC request | 保留整帧含LF≤2 MiB；必须完整编码核验后再写首字节，不能因反斜线再转义溢出 |
| 模板展开 | 保留完整 prompt≤4 MiB；超限准备阶段失败 |
| tools 请求输出 tokens | `max_tokens`显式≤512；未提供时使用≤512的工具默认值，显式超限拒绝；无工具原4096上限不变 |
| 原始工具模式生成 | 累计≤16 KiB，含特殊 token/标记；单 token_piece 也先检查后分配/追加 |
| 正规化输出 | 本轮 arguments≤4096 B，content≤8192 B；总≤12 KiB；JSON实例限制128节点、深度8、单字符串≤2048 B |
| 传输增量 | 单次内容/arguments≤4096 B；元数据独立小首块；编码IPC/HTTP增量≤25 KiB；IPC一般事件≤64 KiB |
| 公共未消费输出账本 | 保持256 KiB，共用原信用与lease；工具保留输出也占账本，详见第7节 |

上下文成立条件只有一个：**同一 prepared 的完整实际模板 token 数 + max_tokens ≤ 当前明确选择的 context**。tools 描述/schema、system、全部历史调用与结果、generation prefix、BOS/特殊 token 全部经过实际模板和 tokenizer 后计数。不能只算 user 字符数、删旧历史或只给第一轮预算。第二次结果回传必须重新准备/计数，超限由调用方压缩或减少工具后重新提交；Nexa 不偷偷删结果。

`usage.prompt_tokens` 来自该 prepared；completion_tokens 计全部采样 token（包括终止、协议标记及未向客户端暴露的失败输出），不按分片或 JSON 长度估算。取消/失败保留真实已知计数；不能伪造0。准备前失败仍按既有零使用规则。

2048 是初次小样本实验设置，不是未来所有模型上限；16 GB 不等于可用内存16 GB。更大模型/context应显示资源风险并记录用户所选值；硬失败来自真实能力/资源/预算，不来自“未在测试名单”。

## 5. 模板、parser 与完成校验

worker 原生线程拥有模型、模板、prepared、parser、sampler；API/Rust core 不直接链接 common 或解释模板。

1. 将完整有界 DTO 转为 common messages/tools，`enable_thinking=false`、`use_jinja=true`；保持既有参数，不向用户内容追加控制词
2. 从本次 apply 保留 prompt、format、parser、generation_prompt、preserved_tokens、additional_stops；grammar/grammar_lazy/triggers 明确记录“未接采样器”，不对外声称 strict
3. tools 模式必须显式加载非空 parser，验证 format 非纯文本回退。用当前模板的合成 call/result 双向 fixture 检查结构能力；若模板不能可靠区分工具/文本，拒绝该请求的 tools 能力，文本能力单独判断。不能按模型名/hash授予能力
4. 核对该模板的 preserved_tokens 与 vocab，将识别工具边界所需特殊 token 保留给 parser；不继续无条件 special=false，也不把所有特殊 token 暴露为文本。模板 additional_stops 按锁定模板语义使用，不混入任意调用方 stop；截断工具结构视失败
5. 首片有界累积 raw output；可以使用 partial parse 观察完整性，但不发送 provisional tool/text 内容。正常模型 EOS/模板正常消息终止后执行 final parse，再做独立完整性及小 schema 校验
6. `COMMON_PEG_PARSE_FLAG_LENIENT` 可能补全/容忍坏输入。**final parse 非空不代表语法完整**：必须核验原始输出具有该模板要求的完整工具边界、arguments 原始片段本身为完整 JSON、parser 未补括号/改写值、没有被忽略的尾随工具结构；无法证明即失败。不能以修复后的 arguments 再 parse 成功作证。不同模板的严格完整性证据要分别适配；尚无该适配的模板可以文本试用，但不得声称工具协议已支持
7. 本轮名称必须在当前工具列表且 tool_choice 允许；调用数必须为1、参数完整且满足声明 schema。非空 reasoning 返回也视工具模式未支持，不能把思考隐藏后当普通答复通过
8. 全部检查通过后才能生成稳定 ID、发送正文。工具调用存在则 finish_reason=tool_calls；没有调用的普通完整答复则 stop。`length` 在首片工具模式一律 `incomplete_generation` 失败，即使 lenient parser 能“补好”；无任何 tool_calls 成功终态

生成 ID 与输入历史 ID 分开：Qwen 类模板通常不生成 OpenAI ID，这是正常情况。Rust adapter 为本次请求/调用index生成 `call_<32位request UUID十六进制>_<index>`，与历史 ID 冲突则失败；同一调用终身复用。不得信任模型给的 ID 覆盖该值。若复用上游 `set_tool_call_ids`，先给稳定 cache/清除模型 ID，避免其默认保留模型 ID。

首片只在最终有效消息上分片，不需要把 compute_diffs 引入实时生产路径。后续实时增量片必须另外证明：完整 name 首次且仅一次发送、arguments 仅追加、UTF-8安全边界、稳定index/ID、partial→final无回撤、parser补全字节不提前发出；diff异常固定脱敏。不能把首片事后分片描述为这些门槛已完成。

## 6. 跨层数据与原子版本迁移

### 6.1 自有 DTO 与 C ABI

共享类型建议：`ToolDefinition`、有限 `ToolParameters`/`PropertySchema`、`AssistantToolCall`、`ToolCallStart`、`ToolArgumentsDelta`；消息以自有 tagged enum 表示四种角色，Rust构造器保留现有 `Message::new(role,text)` 的文本便捷路径。可在 `runtime-types/src/tools.rs` 放有限 schema/关联校验，通过 lib.rs 导出；复用现有锁内 serde_json，无新通用 schema crate。所有 Debug 只给数目/字节数，不能 derive 出正文。

`GenerationRequest` 增加 tools/choice；`RequestEventKind`、`ExecutorEvent`增加 `ToolCallStart { index,id,name }` 和 `ToolArgumentsDelta { index,arguments }`；不带独立工具终态。`FinishReason::ToolCalls` 只在全部调用验证且分片发完后出现。普通 CLI/desktop 不得把工具事件显示为“成功空文本”；当前自身不发送 tools，文本回归维持。

以下以审计基线 shim2 的下一代 **ABI v3** 命名举例，是**新增入口与新类型**；若前置开放模型切片已占用该代，必须在工具实现前统一重定符号后缀、build_info、绑定和验收，不能碰撞复用：

- `air_tool_v3`：name/description/parameters_json 三个 pointer+length
- `air_tool_call_v3`：id/name/arguments 三个 pointer+length
- `air_message_v3`：role、`has_content`、content、calls pointer/count、tool_call_id；None与空串不混淆
- `air_prepare_chat_v3`：messages/tools/choice/options/cancel，返回 prepared 与准确 prompt_tokens
- `air_generate_chat_v3`：结构化同步 callback（text/start/arguments + index）和既有 progress，最终返回新的 `air_usage_v3`；v3 finish值明确为0 stop、1 length、2 cancelled、3 failed、4 tool_calls。v1/v2入口永不返回新值
- callback中的所有字符串仅借用至返回，单块上限同第4节；prepared每次generate调用消耗一次，异常/取消也如此。保持prepared→model→engine销毁、单线程所有权，不加 unsafe Send/Sync
- 工具 ID 由adapter形成并检查，C++不生成另一个不一致ID；C++提供稳定index/name/arguments。旧 `air_message/air_usage/air_generate[_observed]` 布局和文本语义不变；build_info的shim_version变3

新接口不能把完整HTTP JSON塞进native，让两套API验证分叉；schema作为已验证规范化JSON传给common，shim仍复核边界/合法UTF-8/计数/必要语义。Rust与C++重复边界校验用同一 fixture，防止一个接受、另一个静默改写。

### 6.2 一个迁移单元，禁止模糊“各自升级”

从本文35bfd85基线直接实施时，原子单元应为 **私有 IPC2 + shim3 + 同源码 worker/父端/验收器/包 manifest**。但开放模型切片正在先行，可能已改变私有协议：**工具实现开始前必须读取其已收口提交与实际Hello/build_info，冻结“前置版本元组→工具版本元组”**，以该前置IPC代际加1、shim新增ABI下一空闲代际为准；同时替换本文件中的示例v3后缀。尚未冻结这个确切元组就不得开始跨层生产修改，更不能把各层自行升级当策略。llama commit不变。Hello严格比对，不协商回退前代；未知variant/字段、旧worker/新父端、新worker/旧父端一律在接触模型前失败。不能父端仅更新DTO、worker还按文本解释tools。

同步清单包括 runtime-ipc 版本/Hello、worker实际build_info检查、process-host故障测试、adapter FFI/smoke、xtask smoke与package_acceptance、scripts/package_windows.py及其测试、发行文档中的版本断言。运行时公共发现/proof协议 `runtime-types::PROTOCOL_VERSION=1`/`PROOF_PROTOCOL_VERSION=1` 与私有IPC不同：首片保持1，除非另有公共不兼容设计；不要全仓替换所有“1”。桌面嵌套runtime与验收器必须取同一新包，不能混搭旧产物。

开发可按下面任务依赖分次修改，但在原子单元完整前不开放HTTP工具路径、不发布混合版本。源码中的编译失败/测试失败必须解决，不能为保留旧text分支跳过枚举穷举或构造器更新。

## 7. 同一背压账本、取消与唯一终态

既有 TextPermit/credit 在本片语义上扩展为所有 payload 的 permit，可兼容重命名为 OutputPermit；不创建第二套工具队列或无限 Vec。拟议工具模式：

1. 父端在发 Generate 之前，向**现有256 KiB账本**保留96 KiB工具输出 reservation +16 KiB transport scratch。96 KiB覆盖跨worker/adapter/父端校验保留的 raw、parsed、normalized正文与必要副本，不计为消费进度
2. 工具模式至多发放**一个120 KiB** payload credit；`96+16+120=232 KiB`，不照搬文本路径两个credit。普通文本保持原2个credit策略
3. Start/Arguments/Text 都按相同一次性credit进入IPC/actor/SSE；首块的ID/name虽小也不能走免额度旁路。Prepared、usage、最终完成/错误只有固定小字段，按既有控制事件处理
4. raw/parser输出仍保留时不得提前归还96 KiB；正常完成须在原生buffer销毁、cleanup返回之后归还。取消/worker崩溃要等现有cleanup/OS回收确认；未确认保持fail-closed，不能称已释放
5. API保留原affine EventLease直至socket接受或丢弃；序列化后就退credit属于错误。每次delta编码先检查25 KiB，拒绝注入超过界限的name/ID/arguments。接收端维护index/header状态、累计参数字节和完整JSON校验，所需副本也在96 KiB证明内

**96 KiB是待证明的设计额，不是已测内存数。** 实现任务必须列出每一份live正文缓冲的capacity、生命周期、JSON/PEG解析工作区及最坏转义扩张，静态上界加高水位测试证明不超额。若固定parser实际需要超额空间，先缩小该切片上限或重新评审共同账本；不得把剩余内存偷偷放“parser scratch”绕开记账。模型/KV/模板本身内存单独观测，不冒充输出账本已限制总RSS。

断连、显式cancel、三类deadline、慢消费者及shutdown仍沿已有优先路径到native cancel flag，不能等输出队列腾空；准备/parse/发布间检查取消，callback等credit时可取消且有时限。CPU解析有输入界限并受执行deadline/worker终止兜底。唯一内部终态由调度器决定：Completed/Failed/Cancelled三者恰一；shim只返回一次状态，工具事件不是新终态。已发送部分SSE后任何失败不重放，断连不强制发终态，下一请求需能正常运行。

### SSE 形状

每个成功调用先发一个header：`delta.tool_calls:[{index:0,id:"call_..._0",type:"function",function:{name:"lookup_test_color",arguments:""}}]`；随后相同index的 `function.arguments` 字符串增量。后续块不重复name/type/id，不把JSON对象直接放进arguments；禁止index换ID、重复header、无header参数或tool/data在终态后继续。

无文本的工具回复不伪造空字符串文本；只有role帧及工具delta即可。成功尾顺序为 finish_reason=tool_calls、可选 `choices:[]`真实usage、一次 `[DONE]`。工具模式纯文本成功尾为stop。错误沿当前error SSE后关闭，**不追加成功finish或[DONE]**；HTTP已开始后不能再伪装为HTTP400。客户端只能在完整成功终态后执行调用；缺失finish、半流EOF、error、cancel、length均不执行。

## 8. 错误区分与不伪成功

| 场景 | 拟议外部行为 |
| --- | --- |
| 未支持字段/schema/工具非流式/强制choice/非空stop | 开始前HTTP400 unsupported_parameter，安全param |
| 请求坏JSON、重复key、非法ID、关联缺失/重复、超定义限额 | 开始前HTTP400 invalid_request；body整包超限仍413 |
| 当前模板无工具结构/parser完整性支持 | HTTP400 unsupported_tool_calling；不把整个未测试模型称不支持 |
| 模板后token超预算 | HTTP400 context_length_exceeded，不截断 |
| 生成未知工具、参数坏JSON/schema不符、多调用、回撤/解析失败 | invalid_tool_output，未开始流时500；已开始按error SSE；不含正文 |
| 原始/正规化输出字节超限 | tool_output_limit_exceeded，失败不截断为有效调用 |
| 工具模式length/不完整工具边界 | incomplete_generation，失败，不能lenient补全后tool_calls |
| IPC错版本/信用/index/ID/顺序/重复终态 | native_protocol_error内部诊断，按现有安全internal_error外部映射，故障回收 |
| 取消/断连/慢消费/worker丢失 | 保持既有各层码/故障优先级；无自动重试 |

新错误须进入共享枚举/HTTP固定映射与测试；这里的名称是**规划**，不是现有API可用码。工具“执行失败”由Harness编码为tool结果字符串回传，Nexa不伪造执行成功或推断其权限；结果中的命令/指令均是数据。

## 9. 最小无害闭环与静态 fixture

新增fixture均标记 `origin:synthetic-design`、`execution_status:not_run`；设计case的expected不是已运行结果。它们是有效JSON容器，某些字符串故意装坏JSON供未来测试，不能把容器parse成功当业务测试通过：

- [single-tool-cycle.json](../tests/fixtures/tools/single-tool-cycle.json)：一个 `lookup_test_color(code)`，schema只枚举A1/B7/C9；Harness内存表返回固定颜色和fixture标签。无文件、子进程、联网、时间或随机依赖。两次请求及消息关系明确，expected为语义断言，不强制模型逐字输出
- [invalid-cases.json](../tests/fixtures/tools/invalid-cases.json)：输入/输出/生命周期负例与边界生成说明；未来runner将按case生成受控输入，不能说现在已具备runner
- [stream-shapes.json](../tests/fixtures/tools/stream-shapes.json)：成功标准delta、UTF-8任意字节分片重组、空内容、usage、EOF/错误；两index标为将来扩展/首片拒绝，不假装首片多调用通过

Harness测试composition仅注册该工具，关闭默认shell/文件工具、网络与重试；每测试最多2次模型生成/1次执行，超出即失败。执行前另验证allowlist/参数；只在最终tool_calls成功后运行。第二轮把收到的assistant调用（content null、相同ID/arguments）与tool结果按原样回传，不改写成user。语义通过要求第二轮答案使用实际返回的颜色和fixture标签，不能把提前猜中颜色算利用了结果。

### 真实能力/质量与性能门槛

先完成无模型模板/parser和合成wire，再执行真实模型。建议首个实验样本为官方 [Qwen3-1.7B Q8_0 固定文件](https://huggingface.co/Qwen/Qwen3-1.7B-GGUF/blob/90862c4b9d2787eaed51d12237eafdfe7c5f6077/Qwen3-1.7B-Q8_0.gguf)，远端元数据大小1,834,426,016 B、SHA256 `061b54daade076b5d3362dac252678d17da8c68f07560be70818cace6590cb1a`，[同revision许可](https://huggingface.co/Qwen/Qwen3-1.7B-GGUF/blob/90862c4b9d2787eaed51d12237eafdfe7c5f6077/LICENSE)。这些是远端声明，尚未下载/本地hash复算/模板提取/加载，工具效果未知。

该文件只是**实验样本**；工具支持以运行引擎、当前模板可正确编码/严格解析、预算和功能检查为依据，测试矩阵单独记录“已实测”。未实测不等于不支持；不要恢复模型名/hash唯一准入。与开放兼容改造共享能力描述：能尝试加载、文本可用、工具结构可用、实测质量是不同维度。没有完整性适配的工具模板要如实说明具体缺口，不能假称运行过。

初始实验context2048、非思考、现有temperature/top_p/seed显式固定、batch128、threads2仅用于可比样本；i5-8400/16GB再测实际合适线程。不能宣称复现官方包含top_k/presence_penalty的建议。

拟议最小质量集：10个中英/不同枚举值/历史顺序的必需工具场景，各独立运行2次；另10个直接回答或信息不足不应调用的控制场景。每项记录 `正确选择/完整JSON/schema有效/执行次数/正确使用结果/最终完成/耗时`，保留全部失败与分母。首个小闭环门槛为20/20完整成功、控制10/10不误执行、故障注入0次执行；任何未达标都报告实际比例，不用“语法通过”顶替。它仍不是开放任务、默认完整DSH工具集或长期可靠性证明。

记录首次冷载、TTFT（标注文本/工具模式缓冲差异）、prefill/decode、端到端工具回合、至少5次预热后样本、父进程/worker/Harness内存峰值、超预算、取消后下一请求。16GB设备可用内存/磁盘与同跑应用仍需实测；不存在“1.83GB权重所以2GB RAM够”的推断。原0.6B文本回归不省；新样本未完成不放宽实际安全能力检查。

## 10. 可实施任务拆分与最小文件集合

以下全为计划；本次只新建本文件和三份JSON，没有生产修改、模型下载、构建、提交或工作流变更。工具生产任务共同前置条件是开放模型切片及其Windows CI收口，并按第6节冻结实际新版本元组。每一任务单一写入者；Android 14项WIP和独立MNN项目不碰。

| 顺序/依赖 | 任务与最小修改面 | 该步验收 |
| --- | --- | --- |
| T0，无模型 | `native/llama-shim/src/air_llama.cpp` common日志隔离；新增 `native/llama-shim/tests/tool_parser_test.cpp`、CMake测试目标；必要小解析helper | 固定上游模板合成render/parse、空parser拒绝、完整性/坏JSON/截断、日志canary；不依赖模型权重 |
| T1，依赖本契约 | `runtime-types/src/{lib,scheduler,tools}.rs`、必要Cargo.toml引用已有serde_json；`runtime-api/src/dto.rs`有限schema/历史解析与测试，但HTTP功能先保持关闭 | request/关联/限额/未知字段测试；真实pi-ai工具serializer对照；不增加新schema依赖/lock升级 |
| T2，依赖T0/T1 | `native/llama-shim/include/air_llama.h`、`src/air_llama.cpp`、`llama-adapter/src/{ffi,lib}.rs`、`engine-host/src/lib.rs` | v3所有权/回调/错误/准确模板预算/最终校验，保留v2文本；需要原生构建后才可声明通过 |
| T3，依赖T1/T2 | `runtime-core/src/{executor,output,scheduler}.rs`、`runtime-ipc/src/{lib,codec,validate}.rs`、`runtime-worker/src/{lib,tests}.rs`、`process-host/src/lib.rs`及fault_worker/协议测试 | 锁定新版本元组严格握手、96KiB保留证明、单payload信用、控制优先、唯一终态、断连/崩溃/下一请求 |
| T4，依赖T3 | `runtime-api/src/{chat,errors,dto}.rs`及secure_transport_contract；`examples/harness/verify-pi-ai-tools.mjs`（拟新增）；现有CLI/bridge受影响构造器/穷举点 | 此时才开放工具stream；真实HTTP+合成executor+官方客户端；错误不执行；普通文本无回归 |
| T5，与T2–T4原子发布 | adapter native-smoke、`xtask/src/{smoke,package_acceptance}.rs`、`scripts/package_windows.py`/对应测试，相关公共规范/ADR | 所有版本断言同步、管理端native-free、混版失败、Release包身份一致；本计划不直接改CI |
| T6，依赖T0–T5及开放模型能力路径 | 候选实验manifest/fixture身份与真实 `llama-adapter/tests/tool_model.rs`、`runtime-worker/tests/tool_credit.rs`、Harness受限composition（均拟新增） | Windows真实两轮、质量分母、精确模型/模板身份、16GB资源实测；未下载前不宣称准入或完成 |
| T7，后续独立 | 默认DSH工具schema/出站审计、非流式工具、多本轮调用、实时partial/diff、更多模板；strict另设设计 | 对每个新增能力单独扩H06/H07/H08/H12；不顺带启用执行器/全默认工具 |

这些文件是功能必要面的定位，不承诺只有这些文件会产生机械编译改动；全仓 `Message`/`GenerationRequest`/enum构造与match、Hello测试、包版本断言须检查。文件锁、外部依赖升级、生产工作流变更如需发生，另行说明范围。模型开放策略由并行契约统一，本片不建立第二套白名单。

### 现存验收命令（本轮未运行）

以下入口当前存在；运行它们只能验证当时已实现内容，不能凭零匹配的测试过滤器或旧248项通过宣称tools通过：

```sh
cargo fmt --all -- --check
cargo test --locked -p runtime-types -p runtime-core -p model-store -p runtime-ipc -p process-host -p runtime-api -p runtime-cli -p desktop-bridge
cargo clippy --locked -p runtime-types -p runtime-core -p model-store -p runtime-ipc -p process-host -p runtime-api -p runtime-cli -p desktop-bridge --all-targets -- -D warnings
cargo tree --locked -p runtime-api -p runtime-cli -p desktop-bridge --edges normal
node examples/harness/verify-pi-ai.mjs <已准备的隔离客户端根目录>
```

native-free断言另在全新target目录、`AIR_NATIVE_DIR`指向不存在目录，构建父端runtime-cli/API/bridge；既有构建缓存不能证明无native。已有原生/真实回归命令取 [build-lock](build-lock.md) 与 [xtask README](../xtask/README.md)，环境/锁定子模块未就绪时写受阻，不能偷偷升级或下载替代。

### 拟新增命令（当前不存在，不得现在写为通过）

```sh
ctest --test-dir build/native-release -R '^air-tool-parser-test$' --output-on-failure
node examples/harness/verify-pi-ai-tools.mjs <已准备的隔离客户端根目录>
cargo test --locked -p runtime-api --test secure_transport_contract harness_official_pi_ai_tools_roundtrip -- --ignored --nocapture
cargo test --locked -p llama-adapter --test tool_model -- --ignored --test-threads=1 --nocapture
cargo test --locked -p runtime-worker --test tool_credit -- --ignored --test-threads=1 --nocapture
```

这些入口创建后必须确认实际执行数量大于0，Windows多配置CTest加 `-C Release`；真实测试须显式提供已核对的模型路径/实验manifest/context/thread，并让脚本保存可复核但脱敏的结果。DSH工具composition入口尚待固定精确依赖与实现，不编造CLI参数；有了真实入口后再把执行命令写入专属README。

## 11. 验收矩阵与本次交付状态

| 证据层 | 必须覆盖 | 本次状态 |
| --- | --- | --- |
| 文档/fixture | 引用有效、JSON语法、合成标记、正例关系/限额自洽、未伪造token数/模型结果 | 3份JSON语法/合成标记、正例关系/schema/字节限额、SSE拼接、26负例ID、7个本地链接、围栏与空白静态检查通过；未运行工具执行 |
| T0原生无模型 | 模板/schema/history完整render，parser.load，严格完整性，canary隐私，字节/UTF-8边界 | 未执行 |
| Rust合成协议 | 新旧握手、首块/arguments/index/ID、预算、慢读、取消、重复终态/EOF/错误 | 未实现/未执行 |
| 官方pi-ai+Nexa HTTP | 实际工具出站serializer、null history、usage、完整结果、零重试错误 | 未执行；已有文本测试不能代替 |
| Windows真实模型 | 两次完整模板预算、真实tool输出与结果利用、性能/内存、恢复 | 未执行 |
| 官方DSH受限agent | 精确lock/composition、一个无害工具最多执行一次、真实第二回合 | 未执行 |
| 更广模型/默认DSH | 模型可尝试与能力诊断、不同模板、默认工具schema差异、资源行为 | 另阶段，不被本片通过自动覆盖 |

通过后的表述最多分别为“指定版本工具wire子集通过”“所测模型/模板的小型工具闭环通过”。没有上述真实证据前，仍只沿用既有受控文本子集结论。
