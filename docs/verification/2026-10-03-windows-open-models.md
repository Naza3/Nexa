# W02 开放模型加载：实现范围与验证记录

日期：2026-10-03。状态：最终50c9d41已通过WindowsCI37101760025和固定Qwen3真实回归；下载桌面包独立字节闭包复核通过，原字节包于07:03 UTC发送获接受；用户下载、运行与目标机验收仍待确认。此前Linux微补丁前全workspace/之后定向验证按原轮次保留，不追溯修改。本文只授予已执行层级结果，不宣布其他GGUF已真实运行。

## 基线与范围

- 用户提供目标机内存16GB，既有目标Windows10 x64/i5-8400；可用内存和目标CPU实测未知
- 新需求为广泛模型支持，精确验证矩阵不再用作模型名/hash运行白名单；依据[ADR0015](../decisions/0015-open-model-loading-and-validation-evidence.md)
- 开始实现前开发基线为`35bfd85`（Harness窄文本协议测试），当时最新已交付Windows实现为`389eeef`；本轮50c9d41交付见末节
- [CI37087595998](https://github.com/Naza3/Nexa/actions/runs/37087595998)已于02:27 UTC成功并经50项source/hash复核，但仅覆盖35bfd85；开放模型工作区变更不在该提交内，不能继承其通过结论
- 本次不升级llama.cpp、不做GPU/NPU/移动端、不触碰Android B3b WIP；工具契约另行设计，工具功能尚未因此实现

## 已冻结的实施边界（最终WindowsCI通过，目标机待验）

`validated/validation/capabilities`仍是精确历史证据；独立`loadable`决定受控尝试资格，文件hash/结构/metadata/TOCTOU/Windows lease仍须过关。未知架构最终由锁定llama实际load判断，不再用qwen3名称或固定权重hash限制候选。

单文件GGUF v2/v3与已实现tensor结构为边界；未知layout、分片、缺嵌入模板、encoder/diffusion/noncausal或不支持文本输出framing明确拒绝。原始Jinja直接应用，不fallback、不改写消息角色、不静默隐藏控制输出。模板continuation检查会对实际请求重复执行，不仅做乐观能力探针。

managed导入前、manifest与load统一单文件≤16GiB；external原16GiB限额保持。该文件读取/登记预算不是16GB RAM成功保证。metadata context小于默认2048时，当前默认登记仍失败；自动扫描改取min与逐文件诊断属于下一片，用户显式参数不会静默夹紧。

上下文受metadata、131072既有硬限和模板后token预算约束；16GB不能推导速度或131072上下文可用。本切片不新增Job内存硬限制，worker隔离不是完整OOM保障。

目录扫描仍整批原子失败：一个不支持/损坏文件可阻止本轮目录登记；保留旧索引与源文件，不宣称已具备逐文件诊断/部分登记。UI/API将未实测候选与已有精确证据分开；实际load失败不变成历史validated。

内部ResolvedModel字段由validated换为loadable；实际tuple为私有IPC2/shim行为identity3/公共protocol1，C ABI布局仍v2。Engine::new实际build_info和worker Hello都拒绝旧/伪造identity，包manifest与独立验收器同步；锁定llama commit不变。最终50c9d41同源码WindowsCI已完成；目标Win10窗口与其他模型仍待验。

## 本地实际命令与结果（按源码轮次分层）

运行于Linux开发环境，复用固定工具链和独立Release native目录/target；本轮未下载或运行真实GGUF权重。当时源码尚未提交，该Linux阶段不冒充clean CI；后续最终提交及Windows结果见末节。日志隐私/NUL微补丁之后没有再做全workspace重链接，必须保留下列轮次区别，不把定向计数累加成最终全量通过。以下命令的路径用可复现占位表示，源码不保存机器临时路径。

| 检查/命令 | 实际结果 | 验证层级 |
| --- | --- | --- |
| `AIR_NATIVE_DIR=<Release目录> CARGO_TARGET_DIR=<隔离target> CARGO_INCREMENTAL=0 cargo test --locked --workspace` | 退出0；48组结果合计327 pass/0 fail/7 ignored | 日志/NUL微补丁之前的完整本地逻辑/协议/进程回归；ignored含5项真实模型相关、1项官方pi-ai、1项真实包 |
| 同环境`cargo clippy --locked --workspace --all-targets -- -D warnings` | 退出0 | 日志/NUL微补丁前全workspace静态检查 |
| `cargo fmt --all -- --check` | 最终退出0 | 最终源码格式检查 |
| `cmake --build <Release目录> --target air-template-test air_llama air-stream-test --parallel 2` | 退出0 | 固定native三个Release目标编译，不是模型加载 |
| `ctest --test-dir <Release目录> --output-on-failure` | 最终3/3通过、退出0 | 流缓冲、纯模板continuation、真实Engine初始化后的隐私canary；不加载权重 |
| 同native环境`cargo test --locked -p llama-adapter` | 18 pass/0 fail/3真实模型ignored、退出0 | 日志隐私微补丁后adapter定向验证，含编译失败文档测试 |
| `cargo test --locked -p model-store` | 最终40 pass/0 fail/0 ignored、退出0 | 最后NUL模板/key/tensor-name拒绝与普通tokenizer值允许的定向回归 |
| `cargo clippy --locked -p model-store --all-targets -- -D warnings` | 最终退出0 | 最后NUL改动的定向静态检查 |
| `npm run typecheck`、`npm test`、`npm run lint`（apps/desktop） | 全退出0；72测试/7文件通过 | 前端组件/逻辑，不是原生窗口 |
| `python3 scripts/test_windows_package.py` | 退出0；21项、2 skipped | 打包脚本逻辑，平台特定分支未执行，不是Windows产物 |
| `git diff --check` | 最终退出0 | 工作区变更检查 |

### 最终安全微补丁与冻结

真实Engine初始化时强制关闭common日志和Jinja debug；新增privacy-canary覆盖模板构造异常、运行时异常与成功render，检查stdout/stderr不泄露合成canary。新增路径使用受控静态错误，不把原始模板内容或native日志回传给用户。该canary仅证明其覆盖路径，不宣称所有引擎路径的绝对隐私保证。

最终还拒绝GGUF模板、metadata key和tensor name里的NUL，避免C-string截断造成Rust/native看到不同身份；普通tokenizer metadata value里的NUL继续允许，不能全局禁所有字符串NUL。

最终46个源码/测试文件的逐文件hash清单SHA256为`178c83130853daa3b9625688acd3a06d9e0c5af97e8e0e9dd06cf3127b1ef9ad`，写入者停止修改，独立审查无未解阻断。这是代码冻结清单，不是Git提交或完整产品manifest。最后定向验证不能替代微补丁后全workspace；当时本地磁盘剩余约345MiB而停止大型构建；后续50c9d41 WindowsCI完成最终源码门禁，见末节。

### 模板证据必须区分

新增native测试使用锁定llama源码内`models/templates/Qwen-Qwen3-0.6B.jinja`，SHA256为`87a2728cb8dc9fe424d624542f6060ec05a1d285ebbec578bb078900e33396b5`。旧/新render路径在该fixture上的prompt逐字等价与纯文本continuation用例通过。

这不是既有真实GGUF内模板SHA256`57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361`，两者不能互相代替。本地无权重，不授予真实GGUF模板、vocab/token计数、推理、取消或Windows通过结论。UNKNOWN与非EOG CONTROL输出拒绝有代码/单测边界，不能把普通前缀文本按外形误判为特殊token。

### 模板执行与资源限制

Jinja目前没有循环、操作数或中间分配预算；4MiB限制在render完成后才检查，构造期的模板能力探针同样没有内部执行预算。因此不能把输出大小上限称为模板执行时间或峰值内存上限。

Windows产品worker由父进程独立计时：加载使用load_timeout，prepare属于Generate的execution_timeout，默认各300秒；超时发取消，5秒宽限后可调用TerminateJobObject，再至多等待5秒确认并有界清理。取消标志不能保证中断正在执行的Jinja；未确认回收则fail-closed，不宣称已停止或另起worker。该机制避免父端无界等待，不构成Job RAM硬限制，超时前仍可能OOM或拖慢16GB系统；直接进程内使用adapter没有这层父进程强杀保护。

复杂模板的执行/分配预算属于后续待办，不扩入本切片；当前不作全部模板安全或全进程内存安全承诺。

## O01–O12分层矩阵

| ID | 验收范围 | 当前证据/尚缺 |
| --- | --- | --- |
| O01 | 不按架构名/量化标签/hash/name名单限制候选 | 合成manifest与API/store回归通过；多种真实模型未跑 |
| O02 | 历史validated精确证据保持，伪造声明拒绝 | exact_matrix与validation_is_evidence等逻辑回归通过 |
| O03 | 有界metadata/tensor范围/重叠/截断/未知layout/分片/NUL身份边界 | 最终model-store40项定向通过，不是所有GGUF支持声明 |
| O04 | 原始单轮/多轮/system、continuation/EOG与模板日志 | 最终CTest3/3及adapter定向通过；50c9d41另完成固定GGUF嵌入模板及真实推理回归，其他模板仍待验 |
| O05 | load资格、metadata/hash复验、external lease/TOCTOU | 本地存储/生命周期逻辑回归通过；50c9d41 Windows外部文件保护与真实链已回归，用户设备另验 |
| O06 | metadata/131072与16GiB预算、错误资源处理 | 边界逻辑通过；16GB目标机实际内存、真实token与OOM条件未验 |
| O07 | API/bridge/UI三个维度及旧服务缺字段 | 相关Rust回归与前端72项通过；原生窗口未操作 |
| O08 | IPC2/shim identity3与混搭拒绝、取消/终态 | 微补丁前全workspace及之后adapter定向分层通过；50c9d41最终Windows完整产品链通过；下载原字节包独立复核通过 |
| O09 | 混合目录失败不提交半索引 | 既有原子失败回归保留；逐文件诊断及短context自动扫描min未实现 |
| O10 | 固定Qwen3真实模板/生成/取消/重复加载 | Linux当时未跑；50c9d41 Windows独立真实模型步骤通过，实际GGUF模板57f1fd…0361已核 |
| O11 | 精确开放模型提交WindowsCI/包/原生UI/目标机 | 50c9d41 CI/包/解压bridge通过；原字节包独立复核及发送完成、native_window=false，用户下载/运行未确认，目标机未验 |
| O12 | 多桌面样本真实文本/性能/内存/工具能力 | 未执行；1.7B/4B等是样本档，不是产品许可名单 |

## 下一步与未完成条件

独立审查无未解阻断，最终50c9d41 WindowsCI已覆盖固定GGUF真实回归、原生身份、HTTP/CLI、产品包和desktop bridge。下载桌面包独立字节闭包复核及发送已完成；下一步待用户下载/运行后验新包原生窗口/目标Win10/16GB设备，其他模型继续单独实测。模型验证矩阵只新增真实有证据的组合，开放尝试不自动给validated。

无工具协议实现或真实harness工具闭环；未新增GPU/NPU或Android产品能力，14项B3b WIP未改；50c9d41对保留研究线的共享字段机械同步另记于末节。本地结果与末节Windows结果分层；固定模型回归不等于完整W02、多模型、目标设备或工具能力完成。原字节包已向用户发送获接受，具体交付事实见末节；用户下载/运行尚未确认。


## 提交恢复与CI闭包修正（2026-10-03）

开放模型提交 `8522514cac6c287fb43ef9052e677346b24a45ef` 已推送，tree为 `25ad5853acd3ecc8932cd73821e5a54f9b69fdf9`。其Windows运行37101303658与历史MNN运行37101303651当时已启动，后续两者实际均cancelled，不记为失败推理或通过。此前上传中断，代码和冻结清单均保留；恢复后46源码/68暂存路径与14项Android WIP再次核对一致。

复查发现两个确定的静态闭包缺口，补丁不跳过门禁：

- Windows显式CMake构建目标遗漏新 `air-template-test`，而CTest已包含其模板/隐私测试。将该可执行目标加入原列表，保留全量CTest；这是运行到该步骤前发现的配置问题，不伪称已经观察到CI失败日志。
- 共享 `ResolvedModel` 字段改为 `loadable` 后，保留的MNN workspace与原先干净的设备runner仍有旧字段引用。仅同步四处构造/读取；研究候选仍不可被生产resolver加载，测试专用研究分支保持原语义，不开放Android模型，不构建新APK。14项B3b未提交WIP未改，其中旧字段引用仍可能导致该私有工作区不能编译，不能把CI检查推广到WIP。

实际轻量验证：固定Rust1.98.1的mobile workspace `cargo fmt --check`、runner `rustfmt --check`、`git diff --check`及三个CTest到显式目标的静态闭包检查均退出0。未运行本地cargo/native编译；actionlint不可用且未安装替代。后续须以修正提交的实际CI为准。

## 最终50c9d41 Windows CI与交付产物（2026-10-03）

### 精确提交与证据

- 修正提交：[`50c9d41e5de06632b4cbb23de699bd253fec15ac`](https://github.com/Naza3/Nexa/commit/50c9d41e5de06632b4cbb23de699bd253fec15ac)；tree`04dccaf6cc8ba63611bb3abcc162dd26e057d1d1`
- [WindowsCI37101760025](https://github.com/Naza3/Nexa/actions/runs/37101760025)于2026-10-03 06:50 UTC确认success
- 脱敏报告artifact`11268110524`，78,025 bytes，SHA256`f0d2585714d7ecb533fb591442c33faf941640338edcca32da0a3ad264008aed`
- 主代理已下载并独立核验50份inventory/source/大小/hash，精确对应50c9d41；报告只覆盖封闭允许列表，不是任意日志的秘密扫描保证

### Windows实际结果

- 常规Rust聚合329 pass/0 fail/7 ignored。ignored仍如实保留，官方pi-ai可选集成未在此CI执行；真实模型相关另有显式步骤，不以常规ignored冒充已跑
- CTest3/3通过，含流缓冲、原始Jinja continuation与真实Engine初始化后的privacy-canary
- 固定原生构建、真实模型/store/core/worker、HTTP/CLI、完整Release便携/桌面包与仓库外解压desktop bridge均通过
- 实际输入仍是Qwen3-0.6B Q8_0：文件SHA256`9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`，真实GGUF模板SHA256`57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361`；CPU 2线程/context2048/batch128。该真实模板和推理回归不能由此前87a272…纯模板fixture结果代替
- 桌面验收`package_unchanged=true`、`native_window_tested=false`；bridge执行不是原生窗口鼠标/键盘验收，更不是用户i5-8400/16GB表现

### 原始产物、独立复核与交付边界

| 对象 | 大小 | SHA256 | 状态 |
| --- | --- | --- | --- |
| 桌面Actions artifact11267461255 | 8,409,995 bytes | `dfec20d79b5fa1e23ce0629664543011eb2477b9e7be1ed88d50bf059052d4d5` | 已下载；外层包装与内部产品ZIP分别计量 |
| 原始desktop-windows.zip | 9,423,216 bytes | `713cd39d78adeb38e585529f3e188c9a3912090651172e3b268fb21bcab5c47f` | CI内完整包/解压bridge通过；下载原字节包独立闭包复核通过，内容未改，发送获接受 |

独立产物复核通过：750个文件及嵌套runtime的197个文件身份/hash一致；6个AMD64 PE的导入依赖闭包完整；桌面Rust许可540项、嵌入前端npm许可6项、runtime许可186项记录及原文hash已核。3个CRT文件与CI记录的微软签名身份/文件hash一致；本次Linux复核未重新签名，也未独立执行Windows Authenticode验签，不能把记录比对表述为Linux重新验签。

2026-10-03 07:03 UTC以`Nexa-Windows-x64-50c9d41.zip`向用户发送，消息发送获接受。用户文件名与原`desktop-windows.zip`不同，内容保持上述9,423,216 bytes和SHA256不变。发送获接受不等于用户已下载或运行。W01与W02目标机相关门槛继续待用户验收，其他模型、Win10原生窗口、i5-8400/16GB性能、实际离线与长期稳定性均未因CI成功或交付变为已验证；工具调用尚未实现。

### 保留研究回归与取消记录

8522514的Windows37101303658、历史MNN37101303651实际均cancelled，保留上述静态闭包修正原因，不伪造它们已通过。50c9d41的[历史MNN研究回归37101760095](https://github.com/Naza3/Nexa/actions/runs/37101760095)另已成功，仅覆盖共享类型机械迁移和原研究门禁；不是Android App/真机验收，也不恢复Android产品排期，14项旧B3b WIP继续原位保留。
