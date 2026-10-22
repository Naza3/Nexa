# PI Desktop 工具 wire 验证

此目录验证官方 `@earendil-works/pi-ai@1.0.1` 加 PI Desktop `v0.17.0` 自有 patch 的 serializer/parser。用户当前实际 PI 版本仍待确认；不将本目录写成已操作 PI 原生窗口或兼容全部客户端。

## 输入与范围

- `client/package-lock.json` 锁定隔离 npm 依赖，85 项均来自 `registry.npmjs.org`。它不是完整 PI pnpm 依赖树的复刻。
- `upstream-lock.json` 锁定官方来源、每份源码 SHA256、原 patch 和应用后的客户端文件 hash。patch 原件含两份完全相同的 deepseek.json 修改；准备脚本验证相同后只应用一次，不更改任何唯一修改。
- Read/Glob/Grep/asktool/TodoWrite 参数直接从 hash 固定的官方源码表达式提取，应用原 `withExplicitRequired`。Read/Glob/Grep/asktool 的描述为明确合成测试文字；TodoWrite 描述为官方源码。
- `tests/fixtures/pi-desktop/` 保存真实 serializer 出站 JSON，内容全部合成，无真实账号、路径或聊天。并非完整 PI agent 出站抓包。
- PI 与 pi-ai 原许可证保存在 `licenses/`；官方源码、patch 和 node_modules 仅下载到显式指定的仓库外目录，不打进产品。

## 准备与合成测试

需要 Node 24（TypeScript 类型剥离）、npm、Python 和 Git。npm 缓存保存在显式安装目录内，不依赖用户全局缓存。以下准备步骤会联网下载官方源码与 npm 依赖，禁用 npm 安装脚本；不运行 PI、实际系统工具或模型。

```sh
python3 examples/pi-desktop-tools/setup.py /absolute/new/pi-client
node --test examples/pi-desktop-tools/gate.test.mjs
node examples/pi-desktop-tools/verify.mjs /absolute/new/pi-client /absolute/evidence/pi-wire
```

`verify.mjs` 创建临时回环 HTTP 合成服务，逐字节分片响应，验证五个场景：普通文本、五个核心工具声明、工具调用、第二轮工具结果、截断参数流。每次与仓库已审 fixture 全量比较；输出路径只是另存抓取，不静默更新 golden。

普通请求默认含 `store:false` 和 `max_completion_tokens`。通用兼容配置不主动隐藏这些字段；PI patch 默认不发 `strict`。第二轮保留 assistant 的 null content、tool_calls 与 role:tool 的 ID 关联。

重要：官方 parser 在截断流可先产生 `toolcall_end`，随后才是 `error`。测试中的执行门必须等待整个流的成功 `done` 且 `stopReason=toolUse`，再核唯一工具名和精确参数。唯一工具是返回固定值的内存查表；不存在任意函数、文件、命令或网络执行器。截断失败不执行、不重试。

## 真实 Nexa 验证（显式执行）

必须先构建当前 CLI/worker，并准备固定来源且已知完整 SHA256 的真实 GGUF。模型不随此脚本下载；使用用户模型需要先核实实际量化、来源与 hash。Qwen3-0.6B 的结果不授予 Qwen3.5-4B 工具能力。

```sh
python3 examples/pi-desktop-tools/run-real.py \
  --cli /absolute/ai-runtime \
  --client /absolute/pi-client \
  --model /absolute/model.gguf \
  --expected-sha256 EXACT_LOWERCASE_SHA256 \
  --out /absolute/evidence/pi-real.json
```

此驱动只创建自己的临时数据目录，初始化独立凭据和随机回环端口，显式导入/加载该模型，使用真实官方客户端最多两次推理、一次内存工具执行。上下文2048、2线程、batch128、输出预算256；每轮核实际 prompt token。模型自行选择工具；不强迫输出、不替换模板、不修补坏参数或通过重试挑选成功样本。

证据仅保留状态、事件种类、用量、模型 hash/大小与配置，不保留模型正文或令牌。临时凭据通过私有 stdin 传给验证子进程，仅用于自己的回环服务，清理时显式 stop，失败才杀自己的子进程。此验证不是安装包、Windows 原生 UI、PI agent 工具执行链或用户目标设备验收。

## 已执行的开发证据

2026-10-10：Node24.19.0/npm11.9.0，隔离客户端五个合成场景和3个执行门测试通过；Python语法、JS语法检查通过。从新空目录执行完整 setup（固定源码/hash、npm ci、全部唯一 patch）及同5场景独立复现通过。初次安装因环境全局 npm 缓存目录不可写失败，修正为安装目录内独立缓存后通过；未关闭 integrity/hash 校验。真实0.6B与catalog固定Qwen3.5-4B-Q4_K_M两轮闭环均通过，见[脱敏实际证据](verification/2026-10-10-host.json)：分别30.1秒、130.3秒，均为2次真实请求、1次内存查表、结果回传后真实最终答复；正常停止，无重试。0.6B两轮prompt/output为227/22、280/19；4B为346/29、411/19。实际worker握手为protocol5/shim5。模板hash独立从原模型文件提取；随后驱动补入同等有界metadata证据记录，未因此重跑两轮。两轮运行间整合测试重链接了开发二进制，证据内hash明确标为运行后观察，不能冒充启动前锁定的执行文件hash；整合方确认生产源码未变。本轮不是精确发行二进制来源链。当前用户实际文件/PI版本、Windows与原生GUI仍待核验。
