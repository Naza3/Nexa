# 输出摘要、OCR历史和设置持久化验证（2026-10-08）

任务 W02-PERF-2 / W02-OCR-5 / W02-PREF-1，从 `23b52ec` 开始，版本保持0.2.3。范围：聊天/OCR正文下方的性能摘要、最近100条OCR结果、TOML参数/提示词/聊天草稿恢复，以及正常关闭前保存。设计见[ADR0036](../decisions/0036-desktop-results-and-preferences.md)。交付前同步origin/main=`1c3650c`，HEAD已包含main，原开发分支ahead8/behind0，无需合并或重写历史。

## 验证环境与边界

Linux云环境，Rust命令载入 `/workspace/onboarding/nexa-env.sh`，设置 `AIR_NATIVE_DIR=/workspace/Nexa/build/native-ocr`、`CARGO_INCREMENTAL=0`。新增依赖复用工作区既有toml版本，Cargo自动更新根和桌面壳锁文件的desktop-bridge依赖引用，无版本升级。

本次未更改原生推理计时、worker协议或公共SSE，不重新宣称真实模型推理验收。Windows专用Tauri命令及WebView2实际事件、Windows文件分享与用户Win10/i5-8400仍待原生构建和设备验证；Linux壳测试只覆盖可移植逻辑。浏览器验证采用模拟桌面API，与Rust临时目录真实TOML/JSON读写分开。

## 后端与包装检查

| 命令 | 结果 | 证据 |
| --- | --- | --- |
| `cargo test --locked -p desktop-bridge` | 退出0，155通过/0失败 | `/tmp/nexa-persistence-bridge-final.log` |
| `cargo test --locked -p desktop-bridge workbench::` | 最后统一参数范围后9通过/0失败，属于上行子集不加总 | `/tmp/nexa-persistence-workbench-final.log` |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml` | 退出0，38通过/0失败 | `/tmp/nexa-persistence-shell-final.log` |
| `cargo clippy --locked -p desktop-bridge --all-targets -- -D warnings` | 退出0 | `/tmp/nexa-persistence-clippy-final.log` |
| 壳workspace同样 `cargo clippy --locked ... --all-targets -- -D warnings` | 退出0 | `/tmp/nexa-persistence-shell-clippy.log` |
| 两个workspace的 `cargo fmt --check` | 退出0 | 工具会话71888 |
| `cd scripts && python3 -m unittest test_desktop_package` | 退出0，17通过 | `/tmp/nexa-persistence-package-final.log` |

前端全量 `npm test` 43文件936项通过，退出0，日志 `/tmp/nexa-persistence-frontend-full.log`。`npm run typecheck`、`npm run lint`、`npm run build` 均退出0，日志分别为 `/tmp/nexa-persistence-typecheck.log`、`/tmp/nexa-persistence-lint.log`、`/tmp/nexa-persistence-build.log`；生产构建保留既有单JS产物超过500KB提示。

最后补充“保存期间新编辑超UTF-8限额”的提示保护与1条回归，随后workbench专项13项通过，日志 `/tmp/nexa-workbench-tests.log`，属于前述功能的定向检查不加总；typecheck/lint/build按最终文件再次执行。覆盖有效旧回包不得把新无效草稿的“未保存”提示清除，原始输入保留。

第一次全量有两条旧断言在点击关闭后立即同步检查原生close；新增流程必须先异步保存，因此调整为等待实际close调用，仍要求精确一次并保留后续模型/文件操作终态断言。修改后上述全量通过。

后端覆盖：真实Unicode TOML重启读取、SHA revision CAS、首次默认值与损坏文件隔离、字段/编码文件限额、每模型草稿、标准LoadOptions范围、私有文件/符号链接拒绝；OCR101条淘汰、部分结果标记、幂等创建、删除后迟到指标不复活、文本及元数据冲突；文件锁读写并发和后台写入关闭门槛。ChatBatch实际proof实例ID随当前/上一终态保持，错误proof不发令牌且不伪造实例。

壳新增4项关闭nonce状态测试，覆盖重复关闭合并、错误/迟到ACK、过期后拒绝ACK、原生确认框结束前保留pending、清空重试隔离。包装测试核对固定命令、AppManifest与ACL精确一致，无新增通用event/fs/shell权限。

## 过程问题

一次Rust链接SIGBUS实际由云盘构建缓存占满引起。仅执行 `cargo clean -p desktop-bridge` 清理可重建产物，保留源码、模型和原生构建；禁用增量后上述测试通过。一次完整 `cargo metadata --offline` 在未缓存的Windows依赖adler2下载处停止；不将它记录为通过，后续locked实际测试命令均成功。

独立审查发现并修复后台写入未被关闭等待、Windows私有读句柄与原子替换的合作锁缺口、原生标题栏绕过前端保存、关闭中OCR部分正文尚未归档等问题。标题栏转发只使用固定CustomEvent与精确ACK命令；5秒无响应先使nonce过期，再要求明确决定，不能静默退出。

最终复核另外两处竞态已闭环：关闭准备开始同步发布状态，输入/发送/OCR启动边界拒绝新工作，失败后恢复；历史删除先等同ID写入，删除成功清除失败Create/Update重试队列，删除期间的新写入待结果再决定，避免“文件已发布但回执失败”之后重新创建已删除条目。

## 浏览器检查

真实Chromium使用明确标注的模拟桌面API，1440/1024/390宽度均无横向溢出或pageerror。验证聊天/OCR摘要位于输出下方；复制与另存只有原文；OCR正文与匹配指标进入历史；刷新后恢复提示词、每模型参数草稿、输出预算、Markdown视图与结果记录；删除后不再显示；原生关闭事件先ACK、保存未发送草稿，再关闭。浏览器存储fixture仅为重载测试使用localStorage，生产保存由原生TOML/JSON实现，不能混作Windows磁盘验证。

证据：`/tmp/nexa-inline-history-workbench-browser.py`、`/tmp/nexa-inline-history-workbench-browser.json`、`/tmp/nexa-inline-history-{1440,1024,390}.png`、`/tmp/nexa-inline-chat-{1440,1024,390}.png`。主代理另人工查看1440 OCR和390聊天截图；结果区与历史沿用现有卡片和字体样式。

最终typecheck/lint/build均退出0，浏览器开发服务主动停止。`git diff --check`及修改文档231个本地链接/代码围栏检查通过。本次在codex/dev本地提交，不自动推送、创建tag或发布Windows产物。
