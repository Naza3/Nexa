# ADR0038：PI Desktop 的 Nexa OCR 调用插件

2026-10-08，采纳。任务 W04-PI-OCR-1。用户选择 `vastsa/PI-Desktop` 作为前端，要求先完成插件，Nexa 保持模型与推理后端。

## 交付与职责

新增独立包 `integrations/pi-desktop-ocr`，插件 ID `io.github.naza3.nexa-ocr`，版本0.1.0，使用 PI 官方 devkit 生成 `.piplug`。宿主契约固定至 PI Desktop 0.17.0 / `779e16d9c3ca2e966a7ae3db9dd0707243a2831f`；Side Chat `a815de103f3f28f6bbdbe4824753ad761d393284` 提供外观适配参考，保留原 MIT 许可。不是把 PI Agent 图像工具或 Markdown 编辑器当作 OCR 引擎。

插件面板与右侧视图共用同一主进程 controller：选图、显式缩放、顺序队列、Markdown/原文、正文下真实性能摘要、最多100条非空结果、TOML 参数记忆。Nexa 的注册模型、参数档案、服务执行预算、单 actor、独立 worker、公共单图 API 均保持。此调用层不增加工具执行、模型下载或 LAN 图像接口。

## 通信与加载

仅接受 `http://127.0.0.1:<port>`。用户显式选择 Nexa 本机 `api-token` 文件或粘贴令牌，不后台扫描凭据目录。每次连接/开始先调用不带令牌的宿主 `pi.net.fetch(/healthz)`，取得宿主网络授权；后续流式与取消由 Node HTTP 实现，因为固定宿主 fetch 只返回完整正文。

每次认证请求先在同一 TCP 连接完成随机 nonce/HMAC 的 Nexa 服务身份验证，核对实例与协议，随后才发送 Bearer；禁止重定向、代理转发和自动重试。PI Node 插件权限不是 OS 沙箱，本实现将原生网络固定在回环目标，不宣称宿主能检查每个原生 socket。

选择的模型须已在 Nexa 登记主模型与 projector。开启自动加载时，通过既有管理接口在空闲状态加载/切换选中模型；忙、有排队任务、故障或不明确的清理状态暂停提示，不抢占、不恢复 fault。关闭自动加载则要求当前已加载同一模型。此为插件调用策略，不修改 Nexa Chat Completions 自身的模型选择规则。

## 队列、预算与结果

最多20张，按文件选择器返回/分次导入顺序，不按文件名排序，可显式调整。逐图分片上传到插件主进程，内存保存，最终每张≤4MiB、单边≤8192、总像素≤16Mi。renderer 和主进程独立校验格式/尺寸/大小；PNG/JPEG 原生解码由浏览器与 Nexa 继续验证。

每次请求先发送 `image_url` 内容再发送提示词，符合已验证 GLM-OCR 模板。输出预算1–4096，插件等待30–86400秒；服务执行超时独立配置。单项正文≤1MiB；逐图单独绑定 request/instance/model/usage/finish 和性能，不用墙钟总时长冒充 prefill/decode。

失败、停止、输出截断、落盘失败或清理未确认后暂停批次；已输出请求不自动重放。继续仅处理 pending 图片。与 ADR0037 的 Nexa 自有桌面不同，本插件选择截断即暂停，并且关闭面板继续后台工作；这是两个调用端的明确策略差异，不覆盖原桌面规则。

## 持久化和关闭

PI 私有目录保存 `preferences.toml`、`history.toml`；history 最多100条且总文件≤16MiB，超限先移除最旧记录。原图/待运行队列不落盘。每约4秒保存非空部分结果；恢复时把未完成检查点标成不完整，不重新执行。

令牌默认仅内存，显式记住才保存 `credentials.toml`，无额外加密；取消记住/清除令牌删除该文件及自身受控残留临时文件。TOML 原子替换，错误消息不得包含解析源行/识别正文/令牌。禁用或退出时禁止新变更，取消当前自有任务并尽力保存；宿主强制退出可能早于原生清理完成，不保证最后几秒输出必然保存。

Markdown 预览净化 HTML 并禁用主动链接/图片；导出只能通过用户选择的目录写入新的 `.md` 文件。识别正文不进入 PI Agent 工具调用链。

安装与参数见[插件说明](../../integrations/pi-desktop-ocr/README.md)，实际证据及 Windows 条件见[本轮验证](../verification/2026-10-08-pi-desktop-ocr-plugin.md)。
