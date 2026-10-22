# W02-OCR-4 可配置推理执行超时验证

日期：2026-10-08。用户要求将桌面750秒限制改成可配置参数。工作目录 `/workspace/Nexa`，分支 `codex/dev`，起始提交 `40e9bfc`，起始工作树干净。已读取最新 `origin/main`；当前开发分支包含其全部提交，无需合并。版本保持0.2.3，本轮不发行新tag或安装包。

## 实现范围

- 复用 `[runtime].execution_timeout_seconds`，设置 → 资源与校验 → 推理执行超时；默认300秒，可保存1–86400整数。停服、实例锁、CAS保存和重启生效保持；其他runtime字段保留，缺字段的整组更新拒绝，旧TOML缺省及合法超大值读取兼容。
- 桌面聊天/OCR在同一已验证连接上读取effective配置再提交一次请求。响应头等待预算由运行服务的校验、排队、加载、执行秒数加30秒派生；流等待为effective执行秒数加30秒。两处聊天路径均不再使用固定750秒，未生效磁盘值不影响本次请求。非法/缺失预算在提交前失败，给出检查版本、显式停服重启匹配服务的说明。
- 取消、断流、部分输出、单次终态和不自动重放保持。OCR超时显示中文裁图/适当延长时限指引，不建议已设更长时限的用户反向缩短至1800。
- 新设置沿用全局卡片风格、草稿冲突与重置，并纳入恢复缓存白名单。真实浏览器发现初版遗漏 `execution_policy` 会触发恢复缓存警告，已修复并补有效数值/空输入NaN恢复回归。
- 同步[ADR0034](../decisions/0034-configurable-execution-timeout.md)、既有配置ADR、架构、执行规格、JSON Schema/示例和[操作指南](../ocr-windows-cpu.md)。prepare在Started/SSE响应形成前完成；视觉encode、prefill和decode随后执行，全部由执行计时覆盖。

## 实际验证

Rust命令先 `source /workspace/onboarding/nexa-env.sh`，使用现有Rust1.98.1和锁定离线依赖；前端目录为 `apps/desktop`，使用现有Node24.19.0/npm11.9.0。未安装新依赖、改锁或重新构建推理内核。

| 检查 | 结果 / 退出码 | 证据 |
| --- | --- | --- |
| `cargo test --locked --offline -p runtime-api -p runtime-cli -p desktop-bridge --all-targets` | 17组、258通过、0失败、2既有忽略；0 | `/tmp/nexa-timeout-rust.log` |
| `cargo test --locked --offline -p desktop-bridge --test hostile_transport` | 29通过；0；属于上述子集 | `/tmp/nexa-timeout-transport.log` |
| 最终中文错误文案后 `cargo test --locked --offline -p desktop-bridge --lib` | 45通过；0；不重复累加 | `/tmp/nexa-timeout-bridge-final.log` |
| `cargo clippy --locked --offline -p runtime-api -p runtime-cli -p desktop-bridge --all-targets -- -D warnings`；`cargo fmt --all -- --check` | 均0 | `/tmp/nexa-timeout-clippy.log` |
| `cargo test --locked --offline --manifest-path apps/desktop/src-tauri/Cargo.toml` | Linux壳34通过；0 | `/tmp/nexa-timeout-shell.log` |
| 最终共用草稿修复后 `npm test` | 39文件、868通过；0 | `/tmp/nexa-timeout-frontend-final.log` |
| 随后preview非默认校验预算调整：受影响2文件定向测试 | 18通过；0 | 子代理回报；包含600秒校验+1800秒执行得到2850秒等待 |
| `npm run typecheck`、`npm run lint -- --quiet`、`npm run build` | 均0；生产构建保留既有大chunk提示 | `/tmp/nexa-timeout-build-final.log` |
| JSON Schema Draft202012及wire示例检查 | schema合法、9示例通过；非法写入、边界和旧大值契约通过；0 | 同步文档代理实际检查 |
| `git diff --check` | 0 | 提交前检查 |

两个忽略项是隔离的官方pi-ai联调及依赖模型/worker的真实OCR测试，本轮未将其计为通过。此次预算改动不改变原生推理算法，没有以模拟结果声称真实OCR准确率或i5耗时。

## 截断反例与界面

真实TCP协议测试使用同连接server proof及模拟响应。虚拟时间推进超过751秒后，聊天与OCR的响应头等待仍存活并能完成；流式OCR已收到中文部分输出后，751秒仍不终结，到1800秒执行预算加30秒收尾余量后失败并仅取消一次、不重放。测试显式设置saved为1、effective为1800，证明采用运行有效值。GET配置时取消以及缺失、无active、零值、溢出、过短预算均不提交生成。

Chromium运行实际前端，使用明确标记的内存模拟后端：默认300、非法0拒绝、1800草稿取消与保存、运行时禁改、显式停服、保存3600后仍保留旧effective直到启动、重新启动采用3600均通过。localStorage中的4200草稿在刷新后恢复，遇配置变化需用户核对/保留草稿，显式保存后清恢复缓存，不自动保存。

1440、1024、390px三种宽度页面与新卡片均无横向溢出，截图已目视检查，无pageerror。脚本 `/tmp/nexa-execution-timeout-browser.py`，报告 `/tmp/nexa-execution-timeout-browser.json`，截图 `/tmp/nexa-execution-timeout-{1440,1024,390}.png`。复用已有1420预览服务，没有停止其他任务的服务。

## 交付边界与下一步

源码和Linux开发验证已完成；Windows原生构建、打包、用户Win10/i5-8400/16GB上的实际长图识别仍待验证，模拟时钟不代表等待了真实1800秒。桌面和runtime需使用同一新版完整包；旧TOML可读不等于旧运行二进制提供新增预算字段。下一步由包含此提交的新Windows构建验证，并在目标机停服、保存1800秒、启动和重新加载模型后手动识别。
