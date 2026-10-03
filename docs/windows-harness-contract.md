# Windows API 与 DeepSeek Harness 接入契约（规划）

日期：2026-10-03。用户要求API兼容官方 [deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)（命令dsh）。本文件基于官方固定源码审计定义接入路线，尚未安装客户端、实现工具协议或完成真实联调；不能据此宣布兼容。范围见[ADR0014](decisions/0014-windows-desktop-cpu-runtime.md)，排期见[W04](roadmap.md#5-w04-兼容门槛)。

## 1. 版本与低改造接入路线

- 研究基线：[dsh-v0.2.0-rc.2](https://github.com/deepseek-ai/deepseek-harness/releases/tag/dsh-v0.2.0-rc.2)，提交`639ed015397290b3745d163aafe02ffee4aa3f84`，2026-09-29；官方仍为developer preview，版本升级须重验
- [dsh-llm-pi-ai依赖](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/packages/llm/llm-pi-ai/package.json)声明`@earendil-works/pi-ai ^0.87.1`；审计参考v0.87.1提交`f07218c4d4bbc12bef056a7058c3dd49dfe41abe`。这是最低声明版本的源码基线，不是所有rc2安装的实际解析版本；验收必须保存精确包版本与lockfile
- 推荐链路：dsh agent/ctx.llm → `@deepseek-ai/dsh-llm-pi-ai` → 自定义`nexa-local` provider → `api: openai-completions` → 本机Nexa `/v1/chat/completions`
- 默认`deepseek-official`使用Messages：默认根`https://api.deepseek.com/anthropic`并追加`/v1/messages`。仅修改`DEEPSEEK_BASE_URL`不能直连Nexa现有接口。首期不新增Messages网关；若以后明确需要默认adapter零配置兼容，再单列设计和验收

来源：[provider](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/packages/llm/llm-pi-ai/src/provider.ts)、[配置说明](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/packages/llm/llm-pi-ai/README.md)、[默认adapter](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/packages/llm/llm-deepseek/README.md)。

## 2. 客户端配置草案

以下是规划示例，未运行验证。合并到已加载的pi-ai插件配置，不重复注册相同路由。端口/model ID须取实际Nexa实例；API令牌只使用环境变量或获准的凭据引用，不把真实值写进仓库、报告或示例。环境变量名不是已保存凭据的证明。

```yaml
- name: '@deepseek-ai/dsh-llm-pi-ai'
  config:
    providers:
      nexa-local:
        displayName: Nexa Local
        api: openai-completions
        baseURL: http://127.0.0.1:<NEXA_PORT>/v1
        apiKeyEnv: NEXA_API_KEY
        cacheRetention: none
        compat:
          supportsStore: false
          supportsDeveloperRole: false
          supportsReasoningEffort: false
          supportsUsageInStreaming: true
          supportsFinishReason: true
          supportsStrictMode: false
          maxTokensField: max_tokens
        retryPolicy:
          mode: normal
          maxRetries: 0
        models:
          - id: <NEXA_MODEL_ID>
            name: Nexa CPU smoke model
            contextWindow: 2048
            maxTokens: 256
            input: [text]
            reasoningEfforts: false
```

2048/256仅用于当前模型矩阵的保守文本smoke设计，实际输入+工具+历史+输出仍须精确预算，不是通用容量或性能承诺。新模型按自身准入填写。禁止沿用provider缺省的contextWindow262144/maxTokens32768虚报本地能力。

Nexa现有源码已支持`stream_options.include_usage`与choices:[] usage-only帧，故草案保留`supportsUsageInStreaming:true`，仍须用实际dsh/pi-ai通过H05；不能把已有实现写成harness已联调通过。`supportsStrictMode:false`不会删除普通tools，正常dsh agent仍发送tools；当前Nexa严格文本接口会拒绝，因此此配置不能让现版本直接获得完整agent能力。纯文本smoke须使用受控无工具LLM调用/测试composition，真实agent等待工具增量。

`streamIdleTimeoutMs`默认300000；`timeoutMs`与此参数须按CPU冷载、排队/prefill、现有客户端750秒预算协调实测，不能盲目设统一750000或无界延时。dsh默认normal策略最多5次重试，初次联调设0，避免掩盖首错与重复推理；之后独立测试显式选择的重试策略。

来源：[config](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/packages/llm/llm-pi-ai/src/config.ts)、[adapter](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/packages/llm/llm-pi-ai/src/adapter.ts)、[重试插件](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/packages/llm/llm-retry/README.md)。

## 3. 协议差异与实现边界

| 能力 | 当前Nexa | 目标与验收 |
| --- | --- | --- |
| POST `/v1/chat/completions` | 严格文本子集、SSE和非流式已有 | pi-ai实际使用stream=true；从真实出站fixture审查字段，不靠接口同名判兼容 |
| 模型发现/认证 | GET`/v1/models`、Bearer已有 | dsh可手填非空models；发现是可用性增强，非推理必须。正确/错误token、未知model分别验 |
| 默认附加字段 | 未知字段拒绝 | pi-ai未知端点可发store:false；Nexa不接受该未知字段，compat须关闭。max_completion_tokens别名与stream_options.include_usage已有支持；选max_tokens便于显式配置，不能误写二者为实现缺口 |
| 文本messages | system/user/assistant字符串已有 | 固定dsh adapter文本会flatten为字符串；以真实serializer fixture为准，content数组不是首期必要门槛；developer关闭 |
| tools定义 | 未实现 | `tools[].type=function`及name/description/parameters JSON Schema；模板必须理解工具，不是静默忽略字段 |
| 工具历史 | 未实现 | assistant.tool_calls的id/type/name/arguments；role:tool/tool_call_id/content；纯工具assistant.content=null |
| 工具SSE | 未实现 | 按index合并delta.tool_calls，多call的id/type/name首块及arguments增量，finish_reason=tool_calls |
| 文本SSE与结束 | 文本delta/finish/[DONE]已有 | pi-ai依赖finish_reason；UTF-8/JSON任意分片、stop/length、半流EOF均实测；EOF不能当正常stop |
| usage | include_usage与usage-only帧已有实现，客户端联调未验 | 开启include_usage时必须真实token计数，验choices:[]独立usage帧；不能伪造0或漏终态 |
| 思考 | 当前仅准入非思考文本 | 默认reasoningEfforts:false；需要时才验reasoning_content/reasoning/reasoning_text、显示/历史回传与工具组合 |
| Stop/取消 | disconnect及显式cancel已有 | dsh AbortSignal中断HTTP须传到排队、load、prefill、decode、SSE各阶段，资源清理后下一请求可用 |
| 错误/重试 | 已有HTTP/code/SSE错误 | pi-ai部分按状态与文本分类；401/400/上下文/429/503/timeout/worker故障逐项验证，先关重试 |

wire参考：[pi-ai固定源码](https://github.com/earendil-works/pi/blob/f07218c4d4bbc12bef056a7058c3dd49dfe41abe/packages/ai/src/api/openai-completions.ts)、[dsh stream](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/packages/llm/llm-pi-ai/src/stream.ts)。

首期不必增加`/v1/messages`、`/v1/responses`、Files、Embeddings或dsh会话JSON-RPC。该dsh路由尚未映射tool_choice、GenerateOptions.stop会拒绝，response_format/strict JSON schema/grammar工具不是基础接入必要条件，不为兼容提前扩范围。普通tools参数schema不等于已经实现约束解码。

## 4. 工具能力与安全

API字段接收、模板能编码、模型能可靠生成、harness能正确执行是独立门槛。W02选择实用模型，工具能力单独准入；Qwen3-0.6B文本链通过不能替代真实工具闭环。

runtime只生成受控协议数据，工具执行、权限、文件/命令行为归harness。测试先用固定输入输出的无害工具，不因模型产出调用而执行未知命令。对非法工具名、JSON、参数、id和超长结果不伪造成功；请求模板预算包含tools与完整历史，保持有界队列/缓冲和无静默截断。

回环、精确Host、默认拒绝Origin、Bearer与现有服务身份校验保持。先验证dsh宿主进程本地请求，不能为联通开放公网/任意CORS或放宽凭据安全。部分输出后失败不得由Nexa自动重放，客户端策略另行显式验收。

## 5. H01–H12验收矩阵

全部仍待实施/验证；协议fake不能替代模型能力或Windows真机结果。

| ID | 验收 | 完成证据 |
| --- | --- | --- |
| H01 | 精确版本/配置 | dsh/pi-ai实际lockfile、Nexa/llama/模型/模板身份；自定义provider选中正确模型 |
| H02 | 认证/发现 | 正确token通过、缺失/错误拒绝；发现或手填可用，未知模型不偷换 |
| H03 | 真实文本 | Windows CPU中英文/system/多轮，首块至finish_reason/[DONE]完整 |
| H04 | 实际请求字段 | 脱敏golden fixture，字段明确接受/拒绝，不用忽略全部未知字段凑兼容 |
| H05 | SSE健壮性 | UTF-8/JSON分片、空delta、usage-only、length、半途断流及有效结束 |
| H06 | 工具wire契约 | fake仅验证schema、多index/id、arguments分片、null assistant与tool_call_id回传 |
| H07 | 真实无害工具闭环 | 模型call→harness受限工具→结果回传→最终回答；记录精确模型量化和成功率 |
| H08 | 非法工具输出 | 未知工具、坏JSON/参数、缺失重复id、超长参数不误执行/不伪成功 |
| H09 | 可选思考 | 不支持则明确关闭；支持才验独立显示/历史回传/工具组合 |
| H10 | 全阶段Stop | 排队/首load/prefill/decode/SSE中断，唯一终态、无残占、下一请求可用 |
| H11 | 错误与重试 | 状态/文本/code分类、401/400/上下文/429/503/timeout/崩溃/半流，先无重试再独立验策略 |
| H12 | 容量与资源 | prompt+tools+history精确模板预算，队列/背压、大工具结果与冷启动，无静默截断/虚报容量 |

交付分别声明“文本连接已验证”“工具协议测试通过”“指定模型agent闭环已验证”，不得合并为无范围的“兼容DeepSeek”。当前三者均不能由本次只读协议研究授予通过。

## 6. 许可与非目标

[DSH](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/LICENSE)与[pi-ai](https://github.com/earendil-works/pi/blob/f07218c4d4bbc12bef056a7058c3dd49dfe41abe/LICENSE)采用MIT；若分发其代码须保留版权/许可，依赖与模型权重另行核验。本规划未安装、保存凭据、开启服务或改变许可接受状态。

默认Messages adapter的协议网关、其他云provider、完整agent SDK或默认全工具执行均不是本轮新增目标。
