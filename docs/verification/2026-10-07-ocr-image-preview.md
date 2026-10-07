# OCR 选图与预览修复

任务：W02-OCR-3。状态：源码、本地与原生 Windows 构建验证已完成，用户Win10原生界面待验。用户报告加载模型后选图没有预览，且无法识别，要求先完成构建修复再处理。构建解析修复已提交 `843251b`，此任务随后开始。

## 定位与范围

图片进入前端 `<input type=file>` 后，由 `prepareOcrImage` 读取、解码并生成 data URL，之后才允许提交 `ocr_start`。预览之前没有 Rust 文件路径读取；现有 CSP 已允许 data 图片，OCR DTO 与调用字段匹配。

已复现一个确定缺陷：旧代码只接受精确 `File.type=image/png|image/jpeg`，有效 PNG 在空 MIME、`application/octet-stream`、`image/jpg` 时会在读取前被拒；失败使预览为空，识别按钮保持禁用。错误原先出现在整页共用状态中，而预览仍显示普通“请选择图片”，容易被理解为没有反应。尚未取得用户实机 File.type，不能断言用户具体触发条件已被确认。

修复按真实文件头识别 PNG/JPEG，再完整解码并生成规范 data URL；不以扩展名代替内容验证。4 MiB、8192 边长、16 Mi 像素、缩放与后端 MIME/内容校验保持。选图反馈放到预览旁，独立显示准备、错误和成功；重选同文件、快速换图和缩放时使旧异步结果失效，不允许用旧图片启动新识别。

选图入口使用全局按钮样式的“选择图片”，同步触发隐藏文件输入的原生选择窗口；原生输入清空以支持重选同一文件，文件名由独立反馈区显示，避免原生控件显示“未选择文件”与实际状态矛盾。配对/识别忙碌时沿用 fieldset 禁用。

## 本地验证

| 实际命令与范围 | 结果 |
| --- | --- |
| `npm run lint`，目录 `apps/desktop` | 退出 0 |
| `npm test > /tmp/nexa-ocr-image-tests-final.log 2>&1`，同目录 | 退出 0；38 文件、850 项全部通过，55.58 秒 |
| `npm run build`，同目录 | 退出 0；TypeScript 与 Vite 通过，保留已有大 chunk 提示 |
| `python3 /tmp/nexa-ocr-image-browser.py`，仓库根目录 | 退出 0；实际 Chromium FileReader/Image/canvas 与页面提交链路通过 |

850 项中的针对性子集为图片处理28项、页面31项，不重复累加。覆盖有效 PNG/JPEG 的空、通用、错误 MIME，规范 data URL 与原始字节保留，伪造/截断/坏图片拒绝，4 MiB 与像素边界，缩放后精确字节上限，读取异常/中止，同文件重选、快速换图/尺寸变更旧结果隔离及准备中禁止提交。jsdom 不解码图片，单元测试只替换 Image/canvas；文件读取、Blob 与 Base64 处理保持实际实现。测试中两处1×1 PNG语料已校正 CRC，并经独立 CRC/压缩数据审查及真实 Chromium 解码验证。

真实浏览器覆盖9组 PNG/JPEG 与不同 MIME 组合，每组验证预览已解码、请求模型正确、传送字节与原图一致；另验证可见按钮触发 file chooser、2400×960 JPEG 缩放为1600×640 PNG、伪装 SVG 被拒、同文件重复选择。1440/1024/390px 布局均无横向溢出，没有 pageerror。实际证据：`/tmp/nexa-ocr-image-browser.json`，截图 `/tmp/nexa-ocr-image-fixed-{1440,1024,390}.png`；浏览器使用生产图片处理和页面代码，仅后端模型状态/识别回复为模拟。最终只读复审没有剩余阻断。

首次浏览器命令曾在 onboarding Python 虚拟环境下因未安装 Playwright 失败；改用环境已有 Playwright 的默认 Python 后完成上述运行，没有修改项目依赖。先前849项全套及58项子集为增加“选择图片”按钮测试前的结果，以最终850项为准。

## 原生验证与限制

上述浏览器证据验证客户端数据流，不代表 Windows WebView2 原生文件窗口、用户 Win10/i5-8400 实机行为或实际 OCR 内容质量。当前前端改动没有改变后端模型/推理协议；精确提交推送后的 Windows 原生构建与完整产品门禁另行补记。下一步验证新包实际选图出现预览、状态就绪后识别；无法预先保证用户未提供的具体图片能够解码。

精确源码`2770b0937262b903cd4d7391dc6a04e1d6a1d44a`（含前置tag缓存调整）已推送到`codex/dev`；[Actions37636935709](https://github.com/Naza3/Nexa/actions/runs/37636935709)的desktop-build成功，含Windows前端检查和实际Tauri Release；用户新增版本任务后整轮已取消，不作为完整Windows成功。推送前已fetch并确认main仍为`1c3650c`且为当前提交祖先。先前构建修复843的完整成功结果另见CI报告，不作为本提交原生结果。用户随后明确升级0.2.3并创建tag，本任务最终原生验收转到该发行提交。

`v0.2.3`精确提交6309ded的desktop-build于2026-10-07 14:52:36 UTC成功，用时15分56秒。实际Windows日志确认前端38文件/850项通过（测试63.16秒）、桌面壳33项通过，test/clippy与实际Tauri Release全部成功。日志`/tmp/nexa-tag-cache-audit-37638109341/desktop-build.log`；这部分仍不等同用户Win10 WebView2界面实测，整轮发行结果随后补记。

最终6309ded / v0.2.3的五项Windows构建与验收job全部成功，含三格式安装包及生命周期。整体Actions随后因GitHub内部错误未能启动release，补发上传也受认证阻塞；这不改变已完成的Windows产物验证，但不得称公开发行成功。[可下载的已验证CI发行资产](https://github.com/Naza3/Nexa/actions/runs/37638109341/artifacts/11492458105)包含本次OCR修复，已独立校验六文件、版本、精确commit、来源/许可及摘要。详情见[0.2.3发行结果](2026-10-07-release-version-tool.md#023-原生结果与发布阻塞)。用户Windows10/i5-8400的WebView2选图/实际OCR质量仍是单独实机验收，不用浏览器模拟或构建通过替代。
