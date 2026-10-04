# 2026-10-04 模型自动登记与本机基础测试验证

## 范围与状态

基线 `688fe5c7b0e761a91400274e7b80f664b3052506`；锁定 llama.cpp `2149c00f4442dc59302e134a02e4c99d5f7ed9fc` 未变。实现边界见 [ADR0019](../decisions/0019-model-onboarding-and-local-validation.md)。本文为提交前最终源码验证；提交后 Windows 交叉构建、同源组件和产物身份另由构建报告记录，不能预写通过。

## 最终源码检查

以下均实际执行、exit 0：

- `cargo test --workspace --all-targets --locked --offline`：40 组，407 pass / 0 fail / 7 既有 ignored
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
- `cargo fmt --all -- --check` 与 `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check`
- `apps/desktop` 下 `npm run typecheck`、`npm run lint`、`npm test`（193/193）、`npm run build`
- `python3 -B -m unittest discover -s scripts -p 'test_*.py'`：155项，153通过、2 Windows 专用跳过
- `git diff --check`

五 crate 定向273/0/1、bridge30、scheduler42、receipt2和壳契约11项属于聚合子集，不重复加总。三条新私有路由的真实 Hyper TCP FIN/RST 断连共6组合通过，另有独立 target 重跑；不以直接调用 handler 替代传输测试。

覆盖：只读库存不创建服务/写锁；稳定文件发现及未完成文件排除；下载已保存事实、登记和取消顺序；按本次文件身份定位；actor 原子空闲检查与无隐藏排队/切换；模型/参数/运行时变化使证据失效；空/失败/取消不通过；证据写入失败和缺字段拒绝；旧 Passed 不冒称本次成功；Tauri invoke、ACL 和前端 DTO 对齐。

## Linux 真实模型

最终补丁后重新构建 release 的 ai-runtime、ai-runtime-worker、nexa-desktop-harness，并执行：

```sh
nexa-desktop-harness --runtime <绝对路径/ai-runtime> --model <绝对路径/Qwen3-0.6B-Q8_0.gguf>
```

固定 GGUF：639,446,688字节，SHA256 `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`。本轮为独立 fixture 获取并校验，不能算产品 aria2 下载器的新增实测。

- `success=true`、`real_model=true`、`local_text_validation=true`、`offline_inventory=true`
- 参数 context=2048、batch=128、threads=2；首次输出155字节/54 usage tokens，重复输出81字节，取消前部分输出6字节；报告不存生成正文
- 真实短生成后本机记录持久化，停止服务后只读列表仍有对应证明；同实例连接、关闭保持/退出、回收进程和实例锁释放断言通过
- 外部目录报告 `supported=false`：Windows 文件保护、writer/identity race、原生 UI 必须单独验收，不能将这些 false 误称已过

构建使用 Linux GCC 与 CMake4.2.3/Ninja、固定 AVX2 基线；native 与 Windows cross 缓存分离。源码 Rust1.98.1，Node24.19.0/npm11.9.0；本次不迁移工具版本或 pnpm。

## 审查与保留失败事实

独立审查发现并修复：非创建式锁观察、自有登记期间旧库存可见性、观察去抖时间、只绑定当前下载目标、加载源身份绑定、取消/关闭边界、null DTO，以及本次 Loaded/Deferred 记录失败误用旧 Passed。最终复查和独立 bridge/真实传输测试通过。

初次环境检查缺少稀疏 checkout 中两个未改动测试 fixture，按基线原始 Git blob 核对恢复后重跑通过。独立模型获取首次 Python requests 缺依赖，改用现有 curl 并完成大小/hash校验。未删除或掩盖这些前置失败。

## Windows 用户验收与限制

1. 完整解压新包，保留 desktop-windows 内 runtime/download/许可等目录，启动桌面程序
2. 服务停止时打开模型页，应看到已登记列表；被动浏览不得自动启动 API/模型
3. 对已下载或从网站放入配置模型目录的完整 GGUF，进入/刷新模型页并等待稳定观察；运行服务持有目录时显示待登记，显式停止后再刷新
4. 在目录下载时保留“下载后加载并进行基础测试（空闲时）”，保存/登记/测试分别显示；已有加载模型或活动请求时应延期，不替换当前模型
5. 显式加载模型后检查本次短测与本机状态；停止服务、重开列表仍可见历史本机结果；更改参数或文件应失效而非延续旧通过

基础短测只证明该模型/引擎/参数组合完成一次短文本生成，不证明回答质量、长上下文、工具调用、所有模型或长期稳定性。目标 Win10/i5-8400/16GB、Windows窗口/外部文件保护、新版真实下载链路、无开发工具/离线/长期运行仍待实测。

`acceptance-tools` 是开发/发行验收用独立固定模型程序及其运行依赖、清单、hash和许可证；不是模型文件、也不是已经跑好的报告。普通使用保留完整 `desktop-windows` 即可，无须手动运行它来登记或基础测试任意模型。
