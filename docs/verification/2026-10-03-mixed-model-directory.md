# W02 混合模型目录：事务与诊断验证记录

日期：2026-10-03。状态：本机回归及独立源码/事务审查通过，源码已冻结；本切片WindowsCI、完整包、交付与用户目标机待验。以下矩阵分开逻辑、平台特定和真实运行结果。

## 基线与范围

- 开始时HEAD为`f3e1b90ed404a898d116012f44d636eab7d21883`，它仅后置记录50c9d41交付证据
- 已发送产品为`Nexa-Windows-x64-50c9d41.zip`；其[真实CI与交付记录](2026-10-03-windows-open-models.md#最终50c9d41-windows-ci与交付产物2026-10-03)仍有效，但不覆盖本轮partial/逐文件诊断/短context扫描增量
- 按[ADR0016](../decisions/0016-mixed-model-directory-diagnostics.md)实施：混合目录合法集合一次原子替换；全坏保留旧目录/索引/generation；空目录可提交空集合；仅明确内容问题逐文件软拒，既有预算/安全/事务失败仍硬失败
- 诊断仅存当前App生命周期；私有DTO增量不改公共HTTP、worker/native协议身份或library schema。显式import/load参数不降级，自动扫描默认值取min(2048, metadata)
- 不实现工具调用，不改产品preflight边界；混合坏文件验收使用包外目录；Android旧14项WIP与独立项目不属本切片

## C01–C12 分层验收矩阵

| ID | 验证目标 | 当前证据 |
| --- | --- | --- |
| C01 | 好+坏目录一次提交合法集合，partial数量/完整诊断一致 | Linux合成store/bridge及UI用例通过，含未知tensor99内容拒绝 |
| C02 | 全坏不保存、旧目录/index/generation不变；真空目录可空提交 | Linux合成store/bridge用例通过 |
| C03 | 同目录同basename/hash保ID；成功partial移除旧无效external，managed不变 | Linux ID/重复扫描、修复被拒文件和managed兼容回归通过 |
| C04 | 坏文件仍计条目/候选/单文件/总字节；所有parser预算硬失败 | 预算来源及触限回归通过；大set_len仅Unix，跨平台计数helper覆盖16/32GiB/+1/overflow，Windows执行待CI |
| C05 | 内容拒绝先过真实I/O、身份和取消检查；不吞路径/reparse/文件变化 | Linux合成检查与源码审查通过；Windows特有共享访问/reparse条件待CI |
| C06 | 成功及软拒source guard保持到提交/放弃；确定硬失败退出可释放 | 持有/释放控制流审查通过；Windows soft-reject共享访问及mapping未在本机执行 |
| C07 | scan-only核旧目录身份，apply显式换目录；路径未变不掩盖替换 | Linux store/bridge目录替换回归通过，Windows另验 |
| C08 | 取消/timeout/保存失败不发布；提交后durability异常保持真实事实 | 早取消/关闭、deadline、晚取消逻辑通过；原保存分支静态复核与UI模拟通过，真实磁盘write/fsync故障未注入 |
| C09 | 诊断≤512KiB/完整operation≤1MiB，受控basename/message无截断伪成功 | Rust最坏64×1024字节JSON转义完整terminal测试及UI边界通过 |
| C10 | 新DTO partial/file_errors/rejected_files；旧completed缺字段默认兼容 | Rust DTO与UI兼容/反例回归通过 |
| C11 | 扫描短context取min；显式import/load/UI设置不静默夹紧 | 自动扫描与显式导入边界合成回归通过，load/UI默认未改；真实短context模型未跑 |
| C12 | UI诊断生命周期、关闭/重扫/缓存清理、包外混合目录与严格包内规则 | UI组件/逻辑与bridge回归通过，preflight未改；完整产品包、原生窗口/目标Win10未验 |

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

C08证据需保持层级：真实bridge早取消/关闭与ScanControl晚取消有逻辑覆盖；原子保存实现未改，提交后durability分支仅静态复核及既有UI模拟权威刷新测试，本切片未注入真实磁盘write/fsync故障。Windows soft-reject guard/共享访问与mapping仍待精确WindowsCI，原生窗口、目标Win10/i5-8400/16GB、新真实模型、完整包未在本机执行。

## 后续门槛

本机完整workspace 343 pass/0 fail/7 ignored、完整clippy与UI85项通过，独立事务/安全审查无未解阻断；下一步由主代理精确提交并运行WindowsCI，完成后才能授予平台特定与产品链结论。新包独立字节闭包复核和实际发送另记；Win10/i5-8400/16GB、多模型质量/性能、原生窗口、离线与长期运行仍需真实验收，工具调用不在本切片。
