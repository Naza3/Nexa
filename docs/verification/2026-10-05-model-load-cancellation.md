# 2026-10-05：手动停止加载验证

任务：W02/W03 按操作身份停止加载。源码基线 `a5ba7388d23758d31e2bfbc94571ef906fd8e535`，分支 `codex/dev`，本报告是未提交工作树验证，不把旧提交的 Windows CI/交付转授本次改动。

## 修改范围

按[ADR0027](../decisions/0027-owned-model-load-cancellation.md)接通 core 按次令牌、hash/preparation、回环异步管理接口、私有短测取消、bridge/原生命令/ACL，以及模型库、添加和下载后可选自动加载按钮。保留单 actor、LAN 管理隔离、外部文件 guard 与已发布登记事实。新命令共4个（legacy/profile两种 start + next/cancel），不是停止服务。

## 实际检查与结果

- 核心层首轮定向测试：core/API/bridge 基础和新增6项 actor 竞态通过；新增 API4项（操作身份/幂等、短测取消、真实错误优先、自动准入）单独通过
- blocking hash 准备新增用例在取消后继续占有 registry lease，直到真实清理释放；禁止抢先重载
- bridge 新增真实 TCP 故障：start 回执挂住或丢失仍可及时停止、只发一次 start；错误观察不释放 busy；UUID DTO/字段严格性
- ProcessHost 原生进程 fixture：`cargo test -p process-host manual_stop_of_uncooperative --test process_contract` 退出0；`.loading` 原子标记确认已收到 Load 后手动停止，不合作 worker 在5.07秒内终止/reap，PID清空；同 supervisor 随后创建正常 worker 重新加载，旧令牌无影响
- 首轮这个新 fixture 只等PID即取消，实际命中“发Load前取消”而失败；已修正为上述确定性Load接收标记，不将那次失败写成通过
- 前端最终695项测试、typecheck、ESLint、Vite构建通过（主代理/前端工作范围报告）；取消前/中/后、跨页/活动/状态栏、晚回/旧ID、未知回执与重试都包含在该回归
- 最终 `cargo test --workspace --all-targets`：42组539通过、0失败、7既有忽略；`cargo test --workspace --doc` 另7项通过，不计入539
- `cargo clippy --workspace --all-targets -- -D warnings`、root/壳 `cargo fmt -- --check`：退出0；壳 `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets` 31项通过，壳clippy退出0
- `cargo xwin check --manifest-path apps/desktop/src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --all-targets`：退出0；保留既有clang-cl工具族探测warning，仅交叉检查，不是Windows运行
- `python -m unittest discover -s scripts -p 'test_*.py'`：212项，208通过/4平台skip；四个新命令的build/handler/ACL/能力集合闭合
- 独立core/API/probe/hash/bridge/process定向96项及ACL1项通过，都是上述范围子集不再累加；最终另复核Stop与坏IPC/原生Faulted交叉的2项进程用例，测试binary与fault-worker的SHA256前后均稳定，5.07秒通过
- 最后审查发现既有ProcessHost故障归并会在Stop时遮盖真实错误，已窄修为仅worker退出/控制终止归并；收到Load再Cancel后发送坏IPC或NativeFailure的两分支保留Faulted与原错误，PID/reap确认。完整process_contract18项通过
- 第一次聚合clippy发现新回调类型需别名、VerifiedConnection测试构造需补实例字段；已修复并完成上述最终全量重跑，不用局部结果覆盖失败

复用环境：Rust `1.98.1-x86_64-unknown-linux-gnu`、`CARGO_BUILD_JOBS=2`、`CARGO_INCREMENTAL=0`、`CARGO_NET_OFFLINE=true`、已构建锁定 Linux CPU native；未安装新工具、未购买资源、未改网络/防火墙或凭据配置。

## 真实模型与层级

首轮 Linux 真实 Qwen3-0.6B Q8_0，639,446,688 字节，SHA256 `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`；复用已有本地fixture，无模型下载。

`nexa-desktop-harness --runtime <已构建runtime> --model <fixture>` 退出0，`manual_load_stop=true`、`stopped_load_phase=loading`、`reload_after_stop=true`、`local_text_validation=true`、`repeat_text_validation=true`、`offline_inventory=true`、`success=true`。同服务实例停止加载后可重新加载/短测/聊天，最终服务与worker生命周期清理确认；临时数据按既有harness清理。Windows external guard 专属项在 Linux 保持 `supported=false`，未冒称通过。

EngineHost“失败ACK前释放engine”与ProcessHost“真实故障优先”全部最终修改后，重新 `cargo build -p runtime-cli -p runtime-worker -p desktop-bridge --bins`，最终完整harness再次退出0；脱敏原报告见[最终真实模型JSON](2026-10-05-model-load-cancellation-smoke.json)。harness实际断言最终 `status=cancelled`、错误 `request_cancelled`、没有active request或registry工作，旧ID取消回执 `stopping=false`；没有把POST取消ACK当成终态。加载完成后的短测取消允许模型继续驻留，报告不宣称总是强杀进程或一定无驻留；不合作原生加载强杀/reap由独立process fixture另证。

最后这次实际 `stopped_load_phase=loading`、`manual_load_stop/reload_after_stop/success=true`；重新加载的生成与持久化证明、聊天、最终worker/实例清理均通过。Windows Actions的既有desktop smoke现在强制要求新增停止与重载断言，尚未触发本轮新CI。

## 目标机待验

- 原生Windows build/真实模型/完整ZIP与同源许可闭包
- Windows10 x64 / i5-8400 / 16GB 的长hash、长native load、切换时停止、立即重试、小窗/缩放/键盘操作
- 添加/下载后可选加载停止后的已保存/登记事实，以及其他本机/LAN客户端不受误取消
- 系统I/O不可立即打断、资源清理无法确认时的错误呈现；不将“已发送取消”视为释放内存完成

## 完成与交接

开发与本机验证已完成，独立审查无剩余阻断。未提交/推送/触发Actions；由主代理精确暂存本批源码、统一中文提交并按既有授权运行标准Windows Actions。最后检查 `git diff --check` 退出0，无模型权重、凭据或构建二进制纳入源码；最终JSON只有封闭的数字/布尔/阶段/hash观测，不含提示词、生成正文、令牌或完整本机路径。
