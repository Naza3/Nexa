# W04 最小文本互操作验证

此目录冻结官方 DeepSeek Harness 的**无工具文本接入方案**，并实测它所依赖的官方 pi-ai serializer/流解析器。当前 Nexa 文本 API 已满足这个受控 wire，因此本增量不修改生产 DTO、调度器、模型准入或推理代码。

## 固定边界

- DSH：`dsh-v0.2.0-rc.2` / `639ed015397290b3745d163aafe02ffee4aa3f84`，仅源码核对，未执行 DSH adapter/CLI/Web/agent
- pi-ai：`@earendil-works/pi-ai@0.87.1`，参考源码 `f07218c4d4bbc12bef056a7058c3dd49dfe41abe`；实际 npm 包及 OpenAI SDK `6.40.0` 和全部85个依赖固定在独立 [client/package-lock.json](client/package-lock.json)
- DSH 原依赖是 `^0.87.1`，不能将本目录结果推广到其他解析版本。来源、integrity、许可和 SHA256 见 [upstream-lock.json](upstream-lock.json)
- 无真实模型、无 Windows 设备、无 DSH 工具闭环结论。测试 HTTP executor 仅生成固定协议事件，usage=3/1 是合成测试值

## 配置与请求

[config.text-only.yaml](config.text-only.yaml) 是 DSH 插件列表片段，须合并进已有配置并替换端口/注册模型 ID；并非可独立启动完整 Harness 的配置。令牌仅以 `NEXA_API_KEY` 环境变量引用，不把真实值写进仓库。

使用自定义 `nexa-local` provider、`api: openai-completions` 和 `/v1` 根路径；默认 DeepSeek Messages adapter 不能只改 baseURL 接入。profile 显式关闭 store/developer/reasoning/strict，选择 max_tokens、context2048、输出128，重试0。这些是当前文本 smoke 的保守上限，不代表任意输入均能装入2048模板预算。真实模型准入仍由 Nexa 检查。

现有 API 已支持 `stream_options.include_usage`、`choices:[]` usage帧及 `max_completion_tokens` 别名。本配置开启 usage，配套 fake/官方解析测试已有证据；真实模型 token 计数与 Windows 验收仍另计。关闭 strict 不会删除 tools，正常 agent 发 tools 时仍会被拒绝。

固定 DSH 纯文本路径将 user 内容 flatten 成字符串；assistant纯文本也为字符串。因此不为通用 pi-ai 的 content数组扩大本期生产协议。tools（包括空数组）、content数组/null、developer、store（包括false）与未知字段保持明确拒绝。

[fixtures/text-request.json](fixtures/text-request.json) 最初按固定源码构造，随后由实际官方 pi-ai 出站 serializer 和假服务收到的 JSON 逐字段等值验证；不标成实际 DSH 出站 golden。固定中英/system/user/assistant/user文本不含真实用户数据。

## 可执行验证

已有 Rust1.98.1 和 Node >=22.19.0 时运行。下面仅安装独立测试客户端，不修改根依赖、不运行 npm 安装脚本；若环境需安装审批，先按环境要求批准。npm registry 为官方 `https://registry.npmjs.org`，所有依赖的 tarball integrity/许可已锁定。

Windows PowerShell 示例（本轮实际运行平台是 Linux）：

```powershell
$clientRoot = Join-Path $env:TEMP 'nexa-pi-ai-0.87.1'
New-Item -ItemType Directory -Force $clientRoot | Out-Null
Copy-Item examples/harness/client/package.json,examples/harness/client/package-lock.json $clientRoot
npm ci --prefix $clientRoot --ignore-scripts --no-audit --no-fund
node examples/harness/verify-pi-ai.mjs $clientRoot
cargo test --locked -p runtime-api
$env:NEXA_PI_AI_ROOT = $clientRoot
cargo test --locked -p runtime-api --test secure_transport_contract harness_official_pi_ai_consumes_actual_nexa_http -- --ignored --nocapture
```

- 普通 Rust 测试不依赖 npm；官方客户端直连测试显式 `ignore`，只有准备好隔离依赖并指定环境变量后才执行
- 独立脚本启动临时 `127.0.0.1` 假 HTTP 服务，以固定合成 token 验证官方序列化、零重试与流消费。强制向 SDK 交付1字节 body分片，覆盖中文/emoji UTF-8、JSON/SSE边界、空delta、usage-only、stop/length、缺finish、半流EOF、流中error、401/404
- 脚本第二个参数仅供 Rust 测试启动的 Nexa synthetic服务使用；真实 runtime/模型不适用固定回复断言。该模式验证官方客户端 → Nexa真实HTTP安全层/DTO/调度层/SSE → synthetic executor
- 子进程/脚本清理环境中的云凭据、代理和遥测配置，只使用合成测试 token；fetch只能请求精确临时回环端点，拒绝重定向。token不打印、不保存，脚本有15秒硬退出期限
- 测试不会启动 agent、外部命令工具、云模型、模型下载或生产凭据初始化；不分发 node_modules 或上游源码

## 当前结果与剩余验收

2026-10-03，Linux / Rust1.98.1 / Node24.19.0：常规 runtime-api 45项通过，独立官方 pi-ai 7场景通过，官方客户端直连 Nexa synthetic HTTP 单项通过。最终聚合与状态由主线验证记录确认；本目录不将协议fake等同于模型质量。

已经覆盖 H02/H04/H05/H11 的部分窄协议证据。仍缺完整 DSH adapter执行、真实 Windows CPU中英/多轮、实测模型预算/冷载/取消、工具协议/工具模型/agent闭环等，W04不得整体标完成。完整矩阵见 [Windows Harness契约](../../docs/windows-harness-contract.md)。

官方源码：[DSH context](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/packages/llm/llm-pi-ai/src/context.ts)、[DSH配置](https://github.com/deepseek-ai/deepseek-harness/blob/639ed015397290b3745d163aafe02ffee4aa3f84/packages/llm/llm-pi-ai/src/config.ts)、[pi-ai wire](https://github.com/earendil-works/pi/blob/f07218c4d4bbc12bef056a7058c3dd49dfe41abe/packages/ai/src/api/openai-completions.ts)。
