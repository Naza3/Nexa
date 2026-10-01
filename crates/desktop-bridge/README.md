# Desktop bridge（T06）

纯 Rust 的桌面边界。无 Tauri、engine-host、llama-adapter 或原生推理链接；根 workspace 可在没有 WebView 开发库时独立检查。完整 UI 命令与数据字段以 `docs/t06-desktop-contract.md` 和本 crate 的 `dto.rs` 为准。

## 宿主接入

原生壳以固定、已检查的产品布局构造 `Arc<DesktopBridge>`：

- `DesktopBridge::new(data_dir: PathBuf, runtime_executable: PathBuf) -> Result<Self>`
- `default_data_dir() -> Result<PathBuf>` 复用 CLI 默认数据目录解析
- `snapshot()`、`start(initialize_if_missing)`、`models_page(after)`、`import_model(path, model_id)`、`load_model(request)`、`unload_model()` 均为 async
- `settings_save(DesktopPreferences)` 仅写 `desktop-settings.json`；`save_idle(seconds)` 仅在停止且持实例锁时写 `config.toml`，范围 1–86400 秒。两项独立显式保存，各自单文件原子发布
- `chat_start(self: &Arc<Self>, ChatStartRequest)` 同步登记唯一 ID 并原子占位；随后后台启动，实际 `started` 只能来自合法 SSE 角色事件
- `chat_next(Uuid)`、`chat_cancel(Uuid)` 为 async；批次 `terminal` 是 bool，终态来自 `events`；重复终态读取无重复文本
- `stop()` 要求 HTTP shutdown 成功且 `wait_stopped` 确认实例释放；`close()` 读取关闭设置；`close_ui_only()` 明确只关 UI，失败不放行关闭

壳只把原生选择器得到的路径传给 `import_model`，前端只持一次性选择 ID；路径、Token 不通过 bridge DTO 暴露。Windows 原始普通盘符路径在 canonicalize 后受控移除 `\\?\` 盘符前缀，再交现有 API，UNC/URL/最终 symlink 不支持。真实导入 API 返回包装对象，bridge 核对外层与 `model` 内层的 ID、大小和 SHA-256。

Token 复制属于原生壳按钮：调用现有私有 Token 读取校验，直接写系统剪贴板；本 crate 没有 Token getter。

## 安全与关闭边界

所有请求通过 `runtime_cli::client::VerifiedConnection` 的同一 TCP endpoint-bound proof 后发送 Bearer。每项控制请求独立连接，不代理、重定向、自动重连、重放或用失败的 proof 接管既有实例。`start(false)` 永不隐式初始化。启动仅固定原生 Command/参数/路径，stdio 均为 null；Windows 显式 `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`，正常继承外部 Job，不请求 breakaway、不变更 Job 限制或提权，也无失败后换 flags 重试。Nexa 不新建随 UI 关闭而杀掉 runtime 的 Job；默认 UI 退出保留 runtime 的保证受外部宿主 Job 生命周期约束。启动错误仅暴露 OS 数字错误码用于定位。

默认关闭停止接收操作、取消本 UI chat，关闭其 HTTP 流并有限等待终态。关闭与 Import 竞争时丢弃本次导入的连接，触发现有 API `ImportGuard`，随后有限观察 registry 空闲；不取消其他客户端的 registry 操作，也不假装知道其归属。Load/Unload 没有独立操作取消 ID，关闭最多等工作锁 10 秒；未完成返回错误、保留窗口，用户待操作结束后重试。此边界不能报告为所有原生窗口路径已经验收。

SSE 缓冲 ≤64 KiB、事件 ≤32 KiB；传输 frame 按 16 KiB 切片、逐事件解码，因此单个合并 HTTP frame 不是事件大小。队列文本 ≤64 KiB，元事件 ≤32；每批文本 ≤16 KiB；无前端消费进展 10 秒后断流并取消。单回复 ≤256 KiB，会话正文（含当前回复）≤512 KiB、消息（含当前回复）≤128，完整发送 JSON ≤512 KiB。所有上限保留可展示的已有部分并报告未完成，不截断后伪报成功。

完成只来自合法 finish +真实 usage + `[DONE]`；坏 proof、redirect、错误身份、坏 UTF-8/JSON、缺 usage/DONE、突然 EOF 和 SSE error 都不是成功。取消意图先登记，独立 cancel 的 404/202 不是成功或远端已回收证明；本地取消终态发生在本 UI 流关闭后。显式 runtime stop 的远端回收要求仍然更强。

## 验证命令

```sh
source /workspace/shared/nexa-tools/env.sh
cargo test -p desktop-bridge --locked
cargo clippy -p desktop-bridge --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build -p desktop-bridge --release --locked --bin nexa-desktop-harness
nexa-desktop-harness --runtime ABS_AI_RUNTIME --model ABS_GGUF
```

Harness 仅新建唯一临时目录、临时凭据与 listen=0 配置，使用真实 GGUF；输出只有脱敏数值、布尔与路径形状（是否中文/空格），没有正文、Token 或完整目录。覆盖 import/list/load/中文流/取消/再生成/unload、实例复用、两种关闭、设置与显式 stop。它还启动固定内部 `--lifecycle-child` 子进程：子进程创建 runtime、关闭 bridge 并真正退出后，父进程重新证明 API；载入 worker 后另子进程按设置 shutdown，父进程核对锁/记录释放。此证据证明 bridge 进程生命周期，不代替 Tauri 原生窗口 close 事件、WebView2、Windows 用户操作验收。

失败不放宽产品界限。检查日志存 `artifacts/verification/`，不提交临时数据、模型或凭据；证据目录保留首次失败及修复后重跑结果。

## 启动失败诊断（T06 CI 第二轮后）

正常验收失败仍以 exit 1 结束，但 stdout 现在是 ≤4096 字节的闭集 JSON：固定阶段、固定错误类别、白名单 bridge code、可空 i32 OS/进程退出码，以及独立 cleanup 结果。没有任意 message/stderr/路径字段；内部生命周期 child 也必须给出有界、完整且无重复/额外字段的报告。最初失败不会被 cleanup 失败覆盖。`StartupDiagnostics` 是仅 Rust 的数字观测，不属于 invoke 或前端 DTO；每个已接纳 start 尝试先重置它，退出 0 与没有退出观测不同。

`nexa-desktop-harness --probe-launch breakaway` 与 `nexa-desktop-harness --probe-launch inherit_job` 是 Windows 受控对照观察，无模型、无 Token、无 runtime 行为更改。两次调用使用同一 exe、各自唯一私有临时目录。`breakaway` 保留历史失败的三个 Windows creation flags 对照；`inherit_job` 只移除 `CREATE_BREAKAWAY_FROM_JOB`，保留 `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`，与当前产品策略一致。省略策略仍默认 `breakaway`，未知策略拒绝。Job 观测只读调用 `IsProcessInJob` 获取父/子当前是否在任意 Job；两策略都不更改外部 Job 限制或安全设置。

输出 kind 为 `nexa-desktop-launch-probe`、`schema_version=2`，明确记录 `strategy`、父/子 `in_job`（bool/null）和各自查询的 OS 数值错误，绝不输出 Job 句柄或名称。在 250ms 窗口内观察 `tokio::signal::ctrl_c()` 是 pending、收到事件还是返回真实 OS 错误。父端观察至多 10 秒，异常后额外至多 1 秒确认自己的探针子进程退出；未确认回收时保留私有目录并报告 `cleanup_confirmed=false`。已无子进程或退出已确认，才清理有界私有报告。合法观察报告就 exit 0；`success=true` 仅限 pending、child exit 0、所有 OS 错误空、父/子 Job 查询已确认且清理已确认。非 Windows 对两个策略分别报告 `unsupported_platform`。此探针不证明 runtime/worker 生命周期，更不替代 Windows UI 或真实推理验收。继承外部 Job 的子进程仍受其生命周期限制，不能声称超过外部宿主寿命。

所有协议 key、stage/code 和枚举在 `src/bin/harness/report.rs`；Python wrapper 通过源码一致性测试防止白名单漂移。Windows CI 36854351971 的单变量对照已观察到：同 exe 的 breakaway 创建返回 OS 5；inherit_job 父子均在 Job 内且成功创建，`ctrl_c` 在 250ms 窗口内 pending、无错误。当前产品据此选择继承策略，CLI `ctrl_c` 逻辑保持不变。探针不是完整 runtime 启动、真实推理或进程退出后的存活证明，后者仍由完整 Windows 验收判定。
