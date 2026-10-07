# Windows OCR 配对选择修复与页面布局

任务 W02-OCR-2。用户确认“选完主模型后，第二个 mmproj 窗口没有出现”，并要求同批调整 OCR 页面布局、沿用 Nexa 全局视觉风格。基线为 `codex/dev` 的 `bf8c03f308b87803c6f259089cb20ca5f2d6b11c`，包含 main `02c90da`；版本保持 `0.2.1`。

## 根因与修复

旧 `models_pair_pick` 在两个原生对话框之间调用 `regular_file`，将 Windows `fs::canonicalize` 返回的 `\\?\C:\…` 路径传入 `SelectedFile::open`。外部文件的目录策略拒绝这种路径，第一文件即报 `model_directory_unsupported`，无法进入第二窗口。即使跳过此处，后续导入桥接 `local_source` 也拒绝直接传入的特殊路径。旧前端只在页面下方显示笼统“选择模型失败”，丢失具体原因。

- `selection::pair_file` 保留原生对话框的原始 DOS 路径供文件保护与最终导入；canonical 路径仅作重复选择比较。保留本机盘、祖先链接/重解析、普通文件、扩展名、只读句柄保护及一次性 token，不放宽特殊路径规则。
- 导入区域显示选择中、取消、文件名、复制校验中、成功或具体失败与错误码；防止重复点击，失败后重新选择。源路径不传入 WebView。
- 导入后显式刷新模型列表；刷新等待并废弃导入确认前的旧读取，避免成功后下拉框仍缺少新模型。仍需用户选择并显式加载。
- 首次导入从空列表变为非空时保持成功提示展开。选择失败的文案使用输入文件语境，包校验诊断保持不变。
- `selected_pair_same_file` 只来自配对选择，纳入脚本测试既有的 UI 专用错误例外；生产启动诊断白名单不变。

## 页面布局

“1 准备模型”整行展示模型选择、加载参数与可展开导入区；下方“2 选择图片”和“3 识别结果”在宽窗口并排，窄窗口上下排列。继承现有灰紫色、白底卡片、字体、边框、圆角和按钮规则，新增样式只作用于 `.ocr-*`。

预览、图片尺寸、输出预算与提示词集中在图片区；原文/Markdown切换、复制和保存集中在结果区，长结果独立滚动且可键盘聚焦。加载取消、识别停止仍可在任务中操作。详细点击顺序同步到[使用说明](../ocr-windows-cpu.md)。

## 验证与证据

以下最终命令退出码均为 0，Linux 与浏览器证据不替代 Windows 原生执行：

| 检查 | 命令 / 结果 |
| --- | --- |
| OCR 与控制器针对性回归 | `npm test -- --run tests/OcrPage.test.tsx tests/controller.test.ts`，45 项通过；覆盖选择等待、取消、错误、无效双文件结果、导入过程互斥、新模型出现、旧读取失效和首次成功提示可见 |
| 完整前端回归 | 在 `apps/desktop` 执行 `npm test`，38 文件/818 项通过，67.29 秒；45 项针对性检查为其子集，不重复累加 |
| 前端静态检查 | 在 `apps/desktop` 执行 `npm run typecheck`、`npm run lint`，通过 |
| 前端生产构建 | `npm run build`，通过；现有大 chunk 提示保留 |
| 壳格式 | `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check`，通过 |
| 壳单测 | `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --offline`，Linux 34/34 通过 |
| 壳 lint | `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --locked --offline -- -D warnings`，通过 |
| Python 脚本 | `python3 -B -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'`，297 项，292 通过/5 平台跳过 |
| 浏览器布局 | `python3 /tmp/nexa-ocr-layout-check.py`，Chromium + Playwright；1440/1024/390px 均无页面横向溢出，长 Markdown 区域可滚动，无页面异常，配对文件名及状态可见 |
| 独立只读审查 | 原生路径/保护、UI状态及刷新逻辑审查通过；发现的列表刷新与首次成功提示折叠均已修复并加入回归 |

浏览器检查使用明确标示的开发预览与模拟模型/回复，不打开 Windows 原生对话框、不导入真实 GGUF、不证明真实 OCR。临时证据：`/tmp/nexa-ocr-layout-check.json`、`/tmp/nexa-ocr-layout-{1440,1024,390}.png`、`/tmp/nexa-ocr-layout-pair.png`。

新增 Windows 专用回归 `windows_pair_original_paths_pass_leases_and_both_import_source_checks`：证明旧 canonical 输入被拒、两个原始路径可建立文件保护、桥接两项原始源路径通过准入；无运行服务时以 `connection_failed` 为终点，不冒称真实导入成功。既有 Windows CI 的壳 `cargo test --target x86_64-pc-windows-msvc` 将覆盖该项。本机未运行此 Windows 专用测试。

开发中首次脚本回归发现新增 UI 错误码缺少测试分类，补齐后完整通过；新增文案经 rustfmt 修正。前端构建原有大 chunk 提示保留。完整 Windows 构建、双原生窗口及用户 Windows10/i5-8400 实机操作仍待本批新包验证；旧安装包不会因源码推送自行更新。
