# W02 混合模型目录：事务与诊断验证记录

日期：2026-10-03。状态：43ad5c2已通过WindowsCI37108375458、固定GGUF与完整产品包/解压bridge验收；独立下载包复核通过，原字节包于08:45:45 UTC发送获接受；用户下载/运行及目标机仍待确认。此前Linux回归及源码/事务审查结果按原层级保留。以下矩阵分开逻辑、平台特定和真实运行结果。

## 基线与范围

- 开始时HEAD为`f3e1b90ed404a898d116012f44d636eab7d21883`，它仅后置记录50c9d41交付证据
- 本轮开始时已发送产品为`Nexa-Windows-x64-50c9d41.zip`；其[真实CI与交付记录](2026-10-03-windows-open-models.md#最终50c9d41-windows-ci与交付产物2026-10-03)仍有效，但不覆盖本轮partial/逐文件诊断/短context扫描增量
- 按[ADR0016](../decisions/0016-mixed-model-directory-diagnostics.md)实施：混合目录合法集合一次原子替换；全坏保留旧目录/索引/generation；空目录可提交空集合；仅明确内容问题逐文件软拒，既有预算/安全/事务失败仍硬失败
- 诊断仅存当前App生命周期；私有DTO增量不改公共HTTP、worker/native协议身份或library schema。显式import/load参数不降级，自动扫描默认值取min(2048, metadata)
- 不实现工具调用，不改产品preflight边界；混合坏文件验收使用包外目录；Android旧14项WIP与独立项目不属本切片

## C01–C12 分层验收矩阵

| ID | 验证目标 | 当前证据 |
| --- | --- | --- |
| C01 | 好+坏目录一次提交合法集合，partial数量/完整诊断一致 | Linux及43ad5c2 Windows合成store/bridge用例通过，含未知tensor99内容拒绝；UI组件用例通过 |
| C02 | 全坏不保存、旧目录/index/generation不变；真空目录可空提交 | Linux及43ad5c2 Windows合成store/bridge用例通过 |
| C03 | 同目录同basename/hash保ID；成功partial移除旧无效external，managed不变 | Linux及43ad5c2 Windows ID/重复扫描、修复被拒文件和managed兼容回归通过 |
| C04 | 坏文件仍计条目/候选/单文件/总字节；所有parser预算硬失败 | 预算来源及触限回归通过；大set_len仅Unix，跨平台计数helper覆盖16/32GiB/+1/overflow，Windows计数helper已执行通过；Unix大稀疏fixture未移植为Windows大文件试验 |
| C05 | 内容拒绝先过真实I/O、身份和取消检查；不吞路径/reparse/文件变化 | Linux合成检查与源码审查通过；43ad5c2 Windows external17和真实外部文件保护链通过，只授予实际用例范围 |
| C06 | 成功及软拒source guard保持到提交/放弃；确定硬失败退出可释放 | 持有/释放控制流审查通过；43ad5c2 Windows被拒文件guard/共享访问及独立回收通过，mapping实际观察见末节 |
| C07 | scan-only核旧目录身份，apply显式换目录；路径未变不掩盖替换 | Linux及43ad5c2 Windows store/bridge目录替换回归通过 |
| C08 | 取消/timeout/保存失败不发布；提交后durability异常保持真实事实 | 早取消/关闭、deadline、晚取消逻辑通过；原保存分支静态复核与UI模拟通过，真实磁盘write/fsync故障未注入 |
| C09 | 诊断≤512KiB/完整operation≤1MiB，受控basename/message无截断伪成功 | Rust最坏64×1024字节JSON转义完整terminal测试及UI边界通过 |
| C10 | 新DTO partial/file_errors/rejected_files；旧completed缺字段默认兼容 | Rust DTO与UI兼容/反例回归通过 |
| C11 | 扫描短context取min；显式import/load/UI设置不静默夹紧 | 自动扫描与显式导入边界合成回归通过，load/UI默认未改；真实短context模型未跑 |
| C12 | UI诊断生命周期、关闭/重扫/缓存清理、包外混合目录与严格包内规则 | UI组件/逻辑与bridge回归通过，preflight未改；43ad5c2 Windows完整包/解压bridge通过，native_window_tested=false，目标Win10未验 |

## 实际命令与结果

最终检查在Linux开发环境执行，无真实模型下载或native重构建，使用受控并发/独立target。写入者先完成8个native-free crate回归；主代理随后复用与50c9d41相同的native构建身份完成最终全workspace测试与clippy。两者范围分别记录、计数不累加；复用原生构建不等于本轮Windows执行。

| 命令/范围 | 最终结果 | 证据边界 |
| --- | --- | --- |
| `cargo test --locked -p runtime-types -p runtime-core -p runtime-ipc -p model-store -p process-host -p runtime-api -p runtime-cli -p desktop-bridge` | 退出0；266 pass/0 fail/1 ignored | 最终8crate聚合；ignored为需NEXA_PI_AI_ROOT和独立npm依赖的`harness_official_pi_ai_consumes_actual_nexa_http`，本轮未重跑该官方集成 |
| 上述8crate `cargo clippy --locked ... --all-targets -- -D warnings` | 退出0 | 同范围静态检查，不是native或Tauri原生壳构建 |
| `AIR_NATIVE_DIR=<已核同身份native目录> cargo test --locked --workspace` | 退出0；48组343 pass/0 fail/7 ignored | 主代理最终聚合；真实模型/官方pi-ai/真实产品包等ignored如实保留，不能称所有集成已执行 |
| 同native环境`cargo clippy --locked --workspace --all-targets -- -D warnings` | 退出0 | 主代理最终全workspace静态检查 |
| `npm run typecheck`、`npm run lint`、`npm test`、`npm run build`（apps/desktop） | 全退出0；7文件85测试 | 最终UI组件/逻辑/前端生产构建，不是原生窗口 |
| `rustfmt --edition 2024 --check`（本切片8个Rust文件） | 退出0 | 限定源码格式检查 |
| `git diff --check` | 退出0 | 最终变更空白检查 |

较早轮次两crate定向为109 pass/0 fail/0 ignored并通过同范围clippy；之后增加计数helper用例/平台fixture修正，最终已被266项及主代理343项聚合覆盖，109、266与343不累加为更多独立测试。UI85项同样不与之前轮次重复累加。

### 过程中发现并修复的问题

- 首轮在blocking持锁域内发布terminal，前端看到终态立即重扫仍可能遇到desktop_busy，6项目录测试失败；改成持guard完成发布/放弃决定，工作/实例锁释放后再发布terminal，之后定向通过
- 一次新增参数遗漏导致编译失败；补齐后通过，不伪造首次即绿
- 审查发现Windows实磁盘fixture使用非法`<>`basename，已改合法中文名称；纯UI转义fixture与真实文件名测试分开
- 实际十几GiB的`set_len`资源fixture限定Unix稀疏文件，避免Windows物理分配。48GiB总量触限case因此仅Unix执行；生产枚举/核验共用的`add_scan_bytes`另有跨平台16/32GiB边界及overflow单位测试，不能据Linux通过宣称Windows稀疏文件行为已验

### 独立审查与未覆盖条件

源码/事务只读审查通过，无未解阻断；15个源码/测试文件限定diff的SHA256为`35bcec3aaf801872280f991433eccf6762287bfd53967bde616c829c48e0e21a`，该值是审查时diff身份，不是Git提交或产品manifest。ADR0016与目录契约核心语义经交叉核对，文档检查另行记录。

C08证据需保持层级：真实bridge早取消/关闭与ScanControl晚取消有逻辑覆盖；原子保存实现未改，提交后durability分支仅静态复核及既有UI模拟权威刷新测试，本切片未注入真实磁盘write/fsync故障。上述Linux阶段未执行Windows soft-reject guard/共享访问/mapping或完整包；后续43ad5c2 WindowsCI新增实际证据见末节。原生窗口、目标Win10/i5-8400/16GB及新真实模型仍未验。

## 后续门槛

本机完整workspace 343 pass/0 fail/7 ignored、完整clippy与UI85项通过，独立事务/安全审查无未解阻断；后续43ad5c2 WindowsCI已完成下述平台特定与产品链验收。下载原字节包独立复核及发送亦已完成，下一步等待用户设备验收；Win10/i5-8400/16GB、多模型质量/性能、原生窗口、离线与长期运行仍需真实验收，工具调用不在本切片。


## 精确43ad5c2 WindowsCI与交付产物

### 源码与证据身份

- 提交：[`43ad5c27333c2c493bba7c14fbfbcf76288d18b0`](https://github.com/Naza3/Nexa/commit/43ad5c27333c2c493bba7c14fbfbcf76288d18b0)，tree`72f5932f7f555246d2f8890acc34b165fee26b5f`
- [WindowsCI37108375458](https://github.com/Naza3/Nexa/actions/runs/37108375458)，job`111161243743`，最终success
- `evidence-index.json`记录该精确提交、result=pass和封闭允许列表；主代理核验50份inventory/source/大小/hash，文档写入者再次对全部50份文件计算大小/SHA256一致。该机制不等于对任意未列入日志进行秘密扫描

### Windows实际结果与边界

- 常规Rust日志48组汇总344 pass/0 fail/7 ignored。ignored保持真实状态；本轮未在Windows执行官方pi-ai可选集成，真实模型/真实包由另外显式步骤验证，计数不与常规聚合重复加总
- CTest3/3通过：流缓冲、原始模板continuation及真实Engine初始化privacy-canary
- Windows external独立17项通过，含好坏混合、全坏保旧/空目录、短context扫描、parser预算硬失败、坏文件仍计预算、被拒文件guard保持、scan-only目录替换及既有写者拒绝；其中用例也出现在常规聚合，不作为额外17个独立场景累加
- Windows bridge目录9项通过，覆盖partial一次提交、重扫/修复/稳定ID、全坏保旧与空目录、目录替换、早取消/关闭等；Windows guard进程隔离/未知cleanup另2项通过
- 预存可写mapping观察1项通过：本次扫描明确因in-use拒绝，解除mapping后普通写入与重新扫描可用；未在guard持有期间执行映射写入。该观测不证明所有预存映射或同账户恶意写入都不可能改变文件
- 固定原生、真实GGUF/store/core/worker、HTTP/CLI、完整Release便携/桌面包及仓库外解压desktop bridge均通过。输入仍仅Qwen3-0.6B Q8_0，文件SHA256`9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`，真实GGUF模板SHA256`57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361`，CPU2线程/context2048/batch128
- 实际包报告`package_unchanged=true`、`native_window_tested=false`。中文/空格路径和包内受控GGUF/未知DLL/篡改manifest边界通过；这不是原生窗口操作或目标Windows10/i5-8400/16GB验收
- Linux专用大`set_len`/48GiB稀疏总量fixture仍仅Linux执行；Windows实际通过的是共享计数helper与其余适用用例。真实磁盘write/fsync故障注入仍未执行，旧mock或静态复核不升级为该故障的实测

### 原字节产物、独立复核与交付边界

原始`desktop-windows.zip`为9,432,048 bytes，SHA256`5dc8cffe0fd4113b715a989566d481f5ff482099327d036e4768c2af7d66f7b5`。独立下载包复核通过：750个文件与嵌套runtime197个文件的身份/大小/hash一致，实际6个AMD64 PE普通与delay imports闭包完整；桌面Rust许可540项、前端npm许可6项及runtime许可186项记录/原文hash已核。3个CRT文件与CI微软签名身份及hash记录一致，Linux未重新签名或执行新的Windows Authenticode验签。源码身份、API1/私有IPC2/shim行为identity3与50项证据对应，无未解产物阻断。

2026-10-03 08:45:45 UTC以`Nexa-Windows-x64-43ad5c2.zip`向用户发送，消息发送获接受；仅用户文件名不同，原ZIP字节、大小与SHA256保持一致。发送获接受不等于用户已下载或运行，不记录内部交付ID。

其他模型、真实短context模型、工具调用、目标Win10窗口/i5-8400/16GB性能、实际离线与长期稳定性均未因此通过。W01/W02的用户设备相关门槛继续待验；工具调用尚未实现。
