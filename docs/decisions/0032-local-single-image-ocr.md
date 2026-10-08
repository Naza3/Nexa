# ADR0032：本机单图 GLM-OCR 与配对视觉模型

日期：2026-10-07。状态：采纳，源码实现；Windows 新包与用户目标机另验。用户要求在现有 Nexa 中实现上传图片获得 Markdown，并明确按实施顺序推进。使用说明见[Windows CPU OCR](../ocr-windows-cpu.md)，实际证据见[本轮验证](../verification/2026-10-07-local-ocr.md)。

## 决策与范围

后续用户授权的桌面多图队列见[ADR0037](0037-sequential-image-ocr-queue.md)：按导入顺序逐张调用本ADR的单图接口，不改变下面的单请求图片数量与CPU资源边界。

复用现有 API → 单 actor → 独立 worker → 专用原生线程。锁定 llama.cpp `2149c00f4442dc59302e134a02e4c99d5f7ed9fc` 已包含 GLM-OCR/mtmd 支持，不升级引擎、不另起 Python/Ollama 服务。首阶段为一张 PNG/JPEG 与一个非空提示词，模型直接生成 Markdown 原文。默认提示词 `Text Recognition:`。不包含 PDF 分页、版面检测、多区域排序、批量任务、OCR 多轮历史或官方网站完整流水线的质量保证。

GLM-OCR 使用语言 GGUF 与视觉 projector GGUF。显式配对导入将两个文件复制到同一 staging 目录，分别校验 hash、GGUF 结构、文件身份与总空间预算，取消/失败不发布半个模型。完成后同目录原子发布，manifest 的可选 `projector` 记录第二个文件的相对路径、大小、hash、来源与结构信息。旧文本 manifest 不序列化空字段；单文件 external 零复制登记保持。projector 不单独成为语言模型。受管理配对记录必须在已配置 external 库时仍可见；加载前的缓存/身份检查包含两项资产。

`ResolvedModel` 增加可选 projector 路径，显式加载仅在完整解析描述及参数一致时复用驻留模型。受管理目录不支持同 ID 原位更换资产；私自修改文件不能通过原 manifest 的 hash 复验。

## 请求与预算

本机 `POST /v1/chat/completions` 兼容既有字符串文本，并增加下列单图子集，两部分顺序均保留：

```json
{"model":"glm-ocr","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,<BASE64>"}},{"type":"text","text":"Text Recognition:"}]}],"temperature":0,"max_tokens":2048,"stream":true,"stream_options":{"include_usage":true}}
```

仅接受内联 `data:image/png;base64,` / `data:image/jpeg;base64,`，不抓取远程 URL、不读取请求中的文件路径。严格拒绝多图、其他角色/历史、空提示词、未知部分、重复/未知字段和 `detail`。LAN 仍只接受原文本子集。

| 边界 | 上限/行为 |
| --- | --- |
| 解码前文件 | 4 MiB；严格 base64 与 MIME/头部对应 |
| 图片尺寸 | 每边 ≤8192，像素乘积 ≤16,777,216；native 分配前再次检查，完整解码失败明确拒绝 |
| 本机图片 HTTP 封套 | 8 MiB；鉴权先于读取；普通文本/管理仍按原配置 ≤1 MiB |
| 私有 IPC 请求 | 8 MiB；无图片 Generate 仍 ≤2 MiB，包含 LF；事件64 KiB、文本信用预算保持 |
| HTTP 拒绝后收尾 | 最多丢弃8 MiB+64 KiB交通，scratch仍8 KiB、总时间仍1秒，不解析后续请求 |
| token | 模板、真实 mtmd 图像位置和提示词共同计入；加输出预算超窗则失败，不截断输入 |

## 原生与生命周期

新增 additive ABI `air_model_load_with_projector` / `air_prepare_image`，既有 C ABI v2 布局不变。私有 worker protocol=3、shim behavior identity=4；公共 HTTP/proof protocol=1 不变。旧 worker/shim 配对明确拒绝。静态依赖新增 `mtmd`、`vendor-hash` 共10项，`MTMD_VIDEO=OFF`；stb_image 与 miniaudio 的原始内嵌许可纳入完整打包闭包。

mtmd projector、bitmap、chunks、model/context 都由同一推理线程创建/释放，`use_gpu=false`。只有独立取消标志跨线程；mtmd 回调 userdata 指向稳定 model 内部状态，调用结束清空取消指针。视觉编码后再次检查取消，禁止回调提前结束却继续读取未完成 embedding。取消/异常清理图像、KV 与 sampler，保留现有超时、worker 强制回收和 Faulted 显式重载规则。

GLM-OCR 原始模板的单图 framing 与普通聊天 EOS 要求不同，使用独立单图 oracle 校验原始模板渲染结果，并保留图文顺序；不改 GGUF 模板、不放松已有文本模板门槛。仅有 projector 不代表所有视觉模型通用兼容。OCR 配对模型的纯文本请求明确拒绝。

## 桌面与验证标签

原生双文件对话框发放一次性 selection token，UI 不传任意本地路径；导入 HTTP 完成前保持两项文件保护。OCR 与聊天共用一个 ChatSlot 和同一取消/流式消费协议，窗口关闭沿用已有清理。图片仅本机读取，预览和可选缩放在前端；识别原文复用安全 Markdown 显示，原生保存对话框决定 `.md` 目标。

OCR 加载显式展示本次8192上下文、256批次、4线程，可由用户调整且不覆盖已有参数档。输出上限2048与图像共占窗口。配对模型加载跳过不适用的文本短测，只报告 Loaded、generation_pass=false；旧文本 receipt 或固定文本验证矩阵不授予 OCR Passed。质量由真实图片逐条核对，Linux 开发验证不转授 Windows/i5-8400 速度或网站级效果。
