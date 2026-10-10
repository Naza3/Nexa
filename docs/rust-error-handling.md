# Rust 错误处理开发规范

适用于 Nexa 自有 Rust crate 和独立桌面壳。规则约束后续修改；不表示历史代码已全部迁移。产品恢复行为以[错误诊断与显式恢复](error-handling-and-recovery.md)为准，安全与进程不变量以 [AGENTS.md](../AGENTS.md) 为准。

## 1. 分层与类型

- 可预期的 I/O、配置、用户输入、网络、资源不足与取消使用 `Result<T, E>`。`Option<T>` 仅表示正常缺失，不把读取失败转换成“没有配置”。
- 库/领域层使用具名错误类型或现有领域错误枚举。为调用方真正需要区分的情况提供 variant、kind 或稳定类型化原因；实现 `Debug`、`Display`、`std::error::Error`，需要跨线程传播的错误须 `Send + Sync + 'static`。
- 同一领域复用既有错误类型，不创建覆盖所有 crate 的巨型 Error；协议 DTO、调度错误、文件 I/O 错误的归属不混合。`runtime-core` 不依赖 HTTP/Tauri 类型。
- `Box<dyn Error + Send + Sync>` 可用于 CLI 等异构错误汇合的应用边界，不作为可恢复领域 API 的默认类型。需要分类时保留具体错误，不先转成字符串。
- 现有手写标准库实现可以继续使用；`thiserror` 是减少实现样板的可选工具，`anyhow` 是应用边界补充上下文的可选工具，都不是 Rust 强制标准，也不应仅为“统一”引入新依赖。本轮不新增依赖。

## 2. 传播、上下文与根因

- 同类错误用 `?`；跨层转换用明确 `From` 或 `map_err`。增加“哪一步失败”的上下文时保存原始 cause，通过 `Error::source()` 提供，不反复拼接根因文字。
- 不为省事将所有错误映射成 `InvalidInput`、`configuration_invalid` 或 `failed`。仅当边界明确规定脱敏/兼容映射时可收敛，并保留该边界能安全表达的固定类别。
- 禁止通过 `Display`、`to_string()`、字符串包含/前缀/相等来识别内部错误类别；文案可调整，控制流不可依赖文案。允许解析明确版本化的外部错误码协议，但应在接收边界校验并转换，不能将任意远端 message 当错误码。
- 当标准 API 必须保留 `io::Result` 时，可在 `io::Error` 中装入具名错误，并以 `get_ref().downcast_ref` 识别；不要把同名普通 `io::Error::other("...")` 当成该类型。访问 I/O 原生错误使用 `kind()`、`raw_os_error()`，不解析系统本地化提示。
- 遍历任意 `source()` 链必须有界，避免循环或异常实现造成无限遍历。错误链不能成为绕过安全校验或自动恢复的依据。

## 3. 内部错误与公开诊断

- `source()` 用于保留因果关系，不等于允许打印。底层源可能含凭据、URL、提示词、正文及绝对路径，禁止直接序列化到 HTTP、IPC、桌面 DTO、诊断复制或默认日志。
- 公开边界只映射到已有稳定 `code`、兼容 `reason`、字段白名单与允许的数值诊断。未知类型返回固定兜底码，不通过原始 message 兜底。
- 可能携带敏感 cause 的包装错误显式实现脱敏 `Debug`/`Display`；不得无审查地 derive Debug 或打印 `{error:?}`、`{error:#}`。展示文案与源链分开审查。
- 同一失败由拥有最终处理职责的边界记录一次，避免每层“记录后继续抛出”。记录固定操作名、类别、可公开关联标识；不新增全文日志。已有日志设施够用时不为规范化另建日志系统。
- 公开错误码属于兼容契约。改文案不能改变类别、HTTP status、CLI 退出语义或协议字段；新增类别同步客户端映射及兼容测试。

## 4. 结果状态、清理与异步

- 明确区分未执行/未发布、已完成、已发布但持久化未确认、结果未知。错误不是“保证没写入”的同义词。原子发布后 fsync 失败不能提示直接重试或删除目标。
- `JoinError`、进程退出、超时和断连不一律当作业务操作未发生。取消须保留清理与终态语义；确认范围不足时报告结果未知，先查询再让用户决定。
- Drop 只作兜底清理；需要声称“已清理”时显式执行并确认。保留原错误与清理结果，不用清理失败覆盖首要失败。
- `let _ = ...` 只允许有书面理由的 best-effort 操作（如桌面已关闭时写诊断管道、无接收者的终态通知），并说明失败为什么不影响资源/状态；不能据此承诺成功。
- 不自动重放业务请求，不把锁冲突视为旧锁，不绕过身份/ACL/路径校验；可选 LAN bind 降级仅限既定 OS 绑定失败。

## 5. panic、unwrap 与静态门槛

- 外部输入、磁盘、网络、模型、任务取消等正常失败路径禁止用 `unwrap`、`expect`、`panic!` 处理。使用类型约束和 Result，而非捕获 panic 掩盖错误。
- 测试中可使用 `unwrap`/`expect`；生产中只有能说明不变量且有验证的不可失败构造才可用 `expect`，信息必须描述不变量而非用户输入。不得给整个 crate 增加 allow 来掩盖新违规。
- Rust panic/C++ 异常不得穿越 FFI；保留既有边界与进程隔离，不使用 `catch_unwind` 假装能修复 abort、内存破坏或不确定的原生状态。
- 根 workspace 所有项目成员继承 `unused_must_use = "deny"` 和 Clippy `dbg_macro = "deny"`；独立 Tauri workspace 声明相同规则。前者不禁止显式 best-effort 忽略，仍须按上一节审查。
- 不直接全项目开启 unwrap/expect 禁止然后大面积 allow：先区分测试、受证明不变量与真实风险。本轮 lint 不宣称覆盖所有 panic 或字符串识别；这些仍需代码审查与针对性测试。

## 6. 新增/修改错误的验收清单

1. 定义错误所有者、调用方需要分辨的类别及是否可能已产生副作用。
2. 为根因设置 source；验证安全 Display/Debug 与公开 DTO 不含敏感样例。
3. 覆盖成功、各失败类别、未知错误及伪造同名文字不会被识别为具名类型。
4. 跨线程/异步路径覆盖取消、结果未知、清理失败及不重复终态；跨进程覆盖旧/未知错误码。
5. 运行受影响 crate 的 `cargo test --locked`、`cargo clippy --locked --all-targets -- -D warnings` 及 `cargo fmt --check`；同时运行相关协议/前端/安装回归。按 AGENTS 的发布规则另做 Windows 全交叉与原生验收，不能用 Linux 通过替代。
6. 记录未覆盖范围、平台限制和遗留问题；不把本轮两个切片写成“全项目所有错误已解决”。

## 7. 本轮落点与后续迁移

2026-10-10 优先消除两个已有行为中的文本分类：
- runtime-cli 启动错误使用具名原因，保留私有启动报告的固定码、有界解析与闭管道容错。
- token 私有文件错误以类型区分路径安全失败、发布后持久化未确认；配置、设置、工作台与 OCR 历史按类型映射既有外部码。

后续触及现有字符串字段、错误映射或忽略错误时按本规范逐点迁移；完整领域枚举改造、全链路源保留和全项目 panic 审计不是本轮已完成结果。

## 依据

以下为 Rust 官方惯用法；脱敏、固定协议码、结果未知及恢复限制是 Nexa 的项目约束：
- [Rust Book：何时 panic、何时 Result](https://doc.rust-lang.org/book/ch09-03-to-panic-or-not-to-panic.html)
- [标准库 Error trait 与 source](https://doc.rust-lang.org/std/error/trait.Error.html)
- [Rust API Guidelines：有意义、行为良好的错误类型](https://rust-lang.github.io/api-guidelines/interoperability.html#errors-are-meaningful-and-well-behaved-c-good-err)
