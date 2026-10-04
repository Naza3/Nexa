# download-engine：受控 aria2 进程监督器

本 crate 复用锁定且带 Nexa 网络策略补丁的 aria2 进行 HTTP、重定向、重试与 Range；不重写这些协议，也不是通用 aria2 CLI。

## 调用边界

- `DownloadSpec` 只从可信来源适配器产生；桌面前端仍只提交 catalog_id
- `SidecarConfig` 只接受上层已经校验且持续持有身份保护的可执行文件路径。此 crate 不把任意路径自动视为可信 binary
- 上层创建并保护 staging，固定 `payload.part`，持有实例锁。所有正常返回都在根进程、Windows Job 内子进程与两个输出读线程结束后发生
- `transfer` 的 exit 0 只表示传输结束。上层仍须确认 writer stopped、重新取得受保护文件、核对完整大小/SHA256，再用同一 `DownloadControl::begin_publish` 仲裁取消与 no-clobber 发布
- `CleanupUnconfirmed` 是监督线程异常的最后防线；上层不能确认清理、释放保护资源、假报 terminal 或关闭成功
- 不能因 UI 十秒关闭等待超时而 abort 传输 future。后台监督器仍须保留所有租约并完成真实回收

## 进程与输出

Windows 使用 CreateProcessW 的原子 JOB_LIST + HANDLE_LIST + 单 DWORD64 MITIGATION_POLICY；Job 不可继承且设置 KILL_ON_JOB_CLOSE。没有 spawn 后再关联 Job 的窗口。stdin 为 EOF，stdout/stderr 分别持续读取。清洁环境仅保留由 GetWindowsDirectoryW 得到的 SystemRoot，以及从已验证 expected_size 显式生成的 NEXA_PAYLOAD_MAX_BYTES；不继承代理、loader、CA、账户或用户配置变量。

子进程创建时固定设置 `IMAGE_LOAD_PREFER_SYSTEM32_ALWAYS_ON`（`1u64 << 60`，8 字节），不支持或设置失败即拒绝创建，不降级重试。它优先 System32，再应用目录，不是禁止全部非系统模块，也不取代可执行文件/目录身份 pin。没有调用全局设置、修改父进程 DLL 搜索路径或增加其它 always-off 策略。普通桌面 DLL 默认搜索把应用目录放在 System32 前；仅 pin 目录不能阻止新增同名 DLL。此措施针对该搜索次序，不能宣称抵挡 manifest 重定向、系统被篡改或同用户所有攻击。官方文档将 image-load policy 列为 Windows10；具体旧版 Windows10 兼容性和静态 sidecar 真实加载仍需实测，遇不支持保持失败关闭。

依据：[UpdateProcThreadAttribute](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)、[IMAGE_LOAD_POLICY](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-process_mitigation_image_load_policy)、[DLL 搜索顺序](https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-search-order)。Windows 已准备属性数量/8字节值测试，以及查询实际子进程 PreferSystem32Images 位的测试；尚未运行这些 Windows 测试，不把 Linux 回归计入 mitigation 证据。后续还应在隔离目录放置同名测试 DLL 并核对静态 sidecar 的真实系统模块路径。

所有 argv 由 crate 构造：无 shell、RPC、用户 config、netrc、执行 hook、自由 header、凭据或自定义 CA；单任务、单流、不预分配、不自动改名、不覆盖、固定内部重试上限，启用 TLS 校验与 SHA256。跨公网重定向及实际连接目标过滤由固定 sidecar 的源码策略负责，未经补丁的通用 aria2 不满足此契约。

输出以 4096-byte read buffer、2048-byte 当前行上限增量处理 CR/LF，丢弃过长行。只向上层输出已知总量范围内的数值进度和 0–32 数值错误码；不打印或保存原始 stdout/stderr、URL、签名查询、路径或错误正文。进度不是 fsync 或完整 hash 的证明，也不会因 exit 0 人工填成 100%。重试资格只看 OS exit code，不看日志中可被反射的 errorCode。

`attempt` 表示上层启动进程的次数，不表示 aria2 内部重试次数。只有明确 exit 8 时，上层可在确认退出后重置本任务 staging 并选择一次 attempt 2；其它失败不自动全量重试。相同 control 的两小时总 deadline 同时约束两个进程尝试和最后验证，不能重启计时。

## 当前验证与限制

本机 Linux fake 子进程/解析器 16 项测试通过，clippy `-D warnings` 通过。这包括固定 argv、清洁环境、CR/LF/超长输出、双管道背压、取消、超时、数值错误、payload 限额边界/ambient 覆盖及 CAS；不等于 Windows 进程验证或真实公网下载。Unix 进程组实现仅在 `cfg(test)` 编译，非 Windows 正式调用返回不支持。

Windows 特定测试已准备，尚待 Windows 执行：参数引用、原子 Job 子孙回收及被继承管道 EOF。真实补丁 binary、包内身份/依赖、ModelScope/HF 和目标 Windows10 机器均需分别验收。

资源硬限制协议已新增：`NEXA_PAYLOAD_MAX_BYTES` 只能是已验证大小的 canonical ASCII 十进制，范围 4..17179869184，无符号、空白和前导零，绝不继承 ambient 值；非法大小在 spawn 前拒绝。对应 aria2 payload-limit 补丁由独立源码构建任务实现，须在任何网络前拒绝缺失/非法协议，并在 payload write/truncate/allocate/open/init 限额。此 crate 的环境测试不证明该补丁或 Windows 组合已经有效，逐次写入硬门槛仍需独立补丁测试与真实组合验证。文件长度/进度轮询和最终 hash 检查不等价于逐块 gate。

Windows TLS 后端的证书链/吊销联网语义、代理不支持、对应源码/GPL 交付材料，均由源码构建与产品发行记录单独覆盖；此 crate 的进程监督测试不能替代这些证据。
