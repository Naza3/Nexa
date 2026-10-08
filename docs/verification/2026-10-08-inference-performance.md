# 统一推理性能验证（2026-10-08）

任务 W02-PERF-1。范围：原生阶段计时、可靠终态/IPC4、actor 200 条有界历史、本机查询与实例绑定、桌面性能页、ACL和打包版本闭包。产品仍为 0.2.3。本轮从 `0d938d6` 开始；交付前 fetch main=`1c3650c`，HEAD 已包含 main，无冲突或历史重写。设计见 [ADR0035](../decisions/0035-unified-inference-performance.md)，操作见 [使用说明](../inference-performance.md)。

## 环境与边界

Linux 云环境，复用固定 llama/shim4 原生静态构建；`AIR_NATIVE_DIR=/workspace/Nexa/build/native-ocr`。原生 shim 源码、模型和第三方依赖版本未改。Rust 命令先载入 `/workspace/onboarding/nexa-env.sh`。真实模型为已有固定 SHA 的 Qwen3-0.6B-Q8_0 与 GLM-OCR-Q8_0/mmproj Q8 对，图片为合成发票样例，不是用户长图。云端时间不能作为 i5-8400 的速度承诺。

Windows 原生构建、WebView2 窗口、用户 Windows 10/i5-8400/16GB、长期运行以及实际 LAN 双机仍待验证。浏览器验证使用明确标注的模拟后端，不混同真实引擎证据。

## 已执行验证

| 命令/场景 | 结果 | 证据 |
| --- | --- | --- |
| `cargo check --workspace --all-targets` | 退出0；Cargo 自动更新新增测试依赖的锁定引用 | `/tmp/nexa-perf-check.log` |
| `cargo test --locked --workspace --all-targets` | 退出0，596通过/10既有忽略 | `/tmp/nexa-perf-rust.log`、`/tmp/nexa-perf-rust-final.log` |
| 严格 Python `python3 -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'` | 退出0，319项中314通过/5平台跳过；命令/权限清单、IPC包身份纳入 | `/tmp/nexa-perf-python.log` |
| `npm test` | 子代理全量40文件879项通过；随后新增3项，最终定向3文件19项全部通过 | 工具会话24764、97334；未另存日志 |
| `npm run typecheck`、`npm run lint` | 退出0 | 工具会话97334 |
| `npm run build` | 退出0；单JS产物超过500KB的拆包提示保留 | `/tmp/nexa-perf-frontend-build-final.log` |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml` | Linux壳34通过，退出0；不涵盖cfg(windows)执行 | `/tmp/nexa-perf-shell.log` |

核心回归覆盖阶段顺序/累计token/缺测、回调时间扣除、安全整数、IPC非法与重复字段、取消完成竞态、cleanup_unconfirmed优先、200条淘汰、重复request_id不同sequence、拒绝不记、正文/图片/错误全文不入history。API验证原SSE与JSON结构保持、本机鉴权、文本和图片均可查、instance正确、LAN无法查询管理历史。前端验证速率分母、null/零耗时、状态语义、参数范围、过滤/CSV、过期异步、停服、模型淘汰后保留无记录筛选。

## 真实推理

构建命令 `cargo build --locked -p runtime-cli --bin ai-runtime -p runtime-worker --bin ai-runtime-worker` 退出0，日志 `/tmp/nexa-perf-build-final.log`。

- `NEXA_OCR_MODEL=/workspace/models/GLM-OCR/GLM-OCR-Q8_0.gguf NEXA_OCR_PROJECTOR=/workspace/models/GLM-OCR/mmproj-GLM-OCR-Q8_0.gguf NEXA_OCR_IMAGE_DATA_URL=/tmp/nexa-ocr-image.dataurl cargo test --locked -p runtime-cli --test real_ocr -- --ignored --nocapture`：退出0，1通过，71.86秒。真实API/worker/原生OCR三锚点通过；查询usage和实际8192上下文/4线程/256批次一致；取消记录无performance，后续非流式恢复成功且历史共3条。日志 `/tmp/nexa-perf-real-ocr-final.log`。
- `NEXA_TEST_MODEL=/workspace/models/Qwen3/Qwen3-0.6B-Q8_0.gguf NEXA_TEST_THREADS=2 cargo test --locked -p engine-host --test real_runtime -- --ignored --nocapture`：退出0，1通过，17.98秒；所有成功终态要求有效非零prefill/decode指标，取消/队列/预算/空闲重载保持。日志 `/tmp/nexa-perf-real-text-final.log`。
- 同样 Qwen 环境变量，`cargo test --locked -p runtime-worker --test real_credit -- --ignored --nocapture`：退出0，1通过，16.82秒；真实IPC4完整指标、零信用取消及单次信用回收通过。日志 `/tmp/nexa-perf-real-worker.log`。

最终源码 OCR 首条成功记录：输入385 token、输出36 token；prepare=7960µs，prefill=16496048µs，decode=1139352µs，output_callback=1982µs，execution=17675ms。prefill约23.34 token/s、decode约31.60 token/s。仅用于确认测量传递和口径，非受控性能基准或用户硬件预测。

## 浏览器与审查

真实 Chromium 在1440/1024/390宽度完成性能导航、详情、全部筛选、实际剪贴板CSV、停服清空、普通预览无假记录；文档无横向溢出，表格滚动被容器约束，页面无异常。证据 `/tmp/nexa-performance-browser.json`、`/tmp/nexa-performance-browser.py`、`/tmp/nexa-performance-{1440,1024,390}.png`。浏览器专项服务已主动停止。

独立只读审查关闭了历史淘汰后模型筛选值与显示不一致的问题；最终API/bridge实例证明、LAN边界、后端终态/计时和前端异步路径无未解决高影响问题。

最终 `cargo fmt --all --check`、壳workspace fmt、`cargo clippy --locked --workspace --all-targets -- -D warnings`、`git diff --check` 均退出0，修改文档的本地链接全部可解析。第一次clippy发现新增Completed载荷使EventLease错误值超过尺寸门槛，已仅将成功终态performance装箱（每成功测量一次分配），保持TextDelta/信用结构和历史值类型；修复后完整Rust再次596通过/10既有忽略。日志 `/tmp/nexa-perf-clippy-final.log`。追加LAN流式/非流式共用历史、准入拒绝不计入的断言通过，日志 `/tmp/nexa-perf-lan.log`。

最后装箱收口后重新构建父/worker并执行真实文本和OCR链路，上述final日志均退出0；未改变原生计时/IPC4或HTTP内容，真实worker信用测试已在同一协议实现验证。
