# 开发与平台构建锁

日期：2026-10-03。本文保留Windows/开发Linux的实际构建锁与历史证据；Linux是开发/验证环境，不是当前发布平台。已发送产品与当前实现以[当前状态](../PROJECT_STATE.md)为准；aria2下载器有独立来源锁与待验分支，不升级下述llama/推理工具链。Android构建锁已归历史快照，不驱动本轮开发。

## 固定输入

| 项目 | 本轮固定值 |
| --- | --- |
| 原始 Nexa 提交 | `0d3a3cea32b813dad0857f9e1a1e41862ce27168` |
| llama.cpp | `2149c00f4442dc59302e134a02e4c99d5f7ed9fc`，Git submodule |
| Rust | `1.98.1 (48a229cea 2026-09-01)`，edition 2024 |
| CMake / Ninja | 4.4.3 / 1.13.2 |
| C / C++ | GNU 14.2.0，C++17 |
| 开发机 | Linux x86_64，kernel 6.18.44；INTEL XEON PLATINUM 8573C；容器可见 9 CPU、约 9.7 GiB 内存 |
| CPU 选项 | Release、静态库、`GGML_NATIVE=OFF`、OpenMP/CUDA/Vulkan/Metal 关闭 |
| x64 指令集 | 上游固定配置实际启用 SSE4.2、AVX、AVX2、F16C、FMA、BMI2；不宣称兼容所有 x64 CPU |
| Windows 编译器/SDK | MSVC 19.44.35229.0 / SDK 10.0.26100.0；Windows Server 2022 10.0.20348；原生Release、CTest与Rust静态链接已在d3d7cf2的CI通过 |

没有升级跟随 master 的构建脚本。CMake 检查 submodule HEAD，不匹配直接失败。原生模型默认日志被抑制，以避免上游输出完整用户路径；开发基线用合成输入，原始上游日志只留在忽略目录。

## aria2下载组件锁（工作树集成，最终Windows待验）

受控sidecar使用[`third_party/aria2/source-lock.json`](../third_party/aria2/source-lock.json)，不使用系统aria2或用户提供的可执行文件。官方1.37.0源码归档SHA256为`60a420ad7085eb616cb6e2bdf0a7206d68ff3d37fb5a956dc44242eb2f79b66b`；llvm-mingw 20240619/UCRT/LLVM18.1.8工具包SHA256为`27d33157cc252c29ad6f777a96a0d94176fea1b534ff09b5071485def143b90e`。此工具链只用于下载组件，不替换runtime的MSVC链。

| 本地补丁 | SHA256 | 作用 |
| --- | --- | --- |
| network policy | `797bd6205909e0a762ac3973aa2b5b7df1ebbc50eb703123cd9eda0692bda966` | 初始/redirect与实际下载socket HTTPS/443/公网约束 |
| payload limit | `c027a39e84cf8e669c6256643e7498f3fe22aed083feebb36d40d73db9b95800` | 可信size控制写入/截断/分配前单payload硬限 |
| IOFile NUL | `c052132bc5e94cc2187544d89c954fd73a6ad94f19c6cf70684cc66f26f7fb86` | 本地修复`strlen==0`后`len-1`下溢，非官方release已修声明 |

构建使用系统SChannel/UCRT，静态链接锁定LLVM/MinGW支持库；不随包引入OpenSSL/CA bundle，保留OS证书链与吊销语义。完整对应源码归档、原始上游/补丁/锁/脚本与版权许可随组件，实际来源/PE/字节身份绑定同一产品提交；测试EXE与fixture/log不入产品。

辅助分支`codex/nexa-aria2-build`的`01db921354be64b67b38f1afdaa23e09dc79c4bd`、tree`a8724e0ce3f07353de6cbd25f57c84683e89e792`在[CI37138664930](https://github.com/Naza3/Nexa/actions/runs/37138664930)已通过Linux构建/fixture；Windows job无runner、steps为空而失败，已于17:05请求重跑，尚无该版Windows结果。Linux交叉构建出AMD64 PE不等于Windows实际运行；旧18bb仅有68/26/4测试通过后被错误WinTLS banner断言终止，后续未执行不追认为通过。原版Windows4 fixture、旧下载器CI与本三补丁构建均分开记录，见[aria2验证页](verification/2026-10-03-aria2-download-engine.md)。

## 已实现的原生入口

```sh
git submodule update --init --recursive
cmake -S native/llama-shim -B build/native-release -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build/native-release --target air_llama air-stream-test llama-completion llama-bench -j 4
ctest --test-dir build/native-release --output-on-failure
```

本 commit 的上游 CLI 聊天工具拆分后，`llama-cli` 受 server 构建开关约束；此 CPU 基线使用实际存在的 `llama-completion` 和 `llama-bench`。第一次请求不存在的 `llama-cli` 目标失败后改为上述真实目标，未更换 llama commit。

`llama-completion --reasoning off` 的初始 prompt 分支没有把关闭思考传给 `common_chat_templates_inputs`。直接运行会出现思考输出，不能算非思考验收。上游非思考基线使用 GGUF 中的原始 Jinja 模板、`enable_thinking=false` 和固定合成 messages 渲染后，通过 `--no-conversation --file` 输入；不拼接 `/no_think`、不删除输出标签。自有 shim 则直接调用固定版本的 `common_chat_templates_apply` 并关闭思考。

该行为可从固定版本 [completion.cpp](https://github.com/ggml-org/llama.cpp/blob/2149c00f4442dc59302e134a02e4c99d5f7ed9fc/tools/completion/completion.cpp) 的初始 prompt 构造分支重查。完整上游 help 与合成基线日志位于本机 `artifacts/verification/`。

## Rust 原生链接

`llama-adapter` 负责单线程拥有原生对象，`runtime-types` 无原生依赖。`AIR_NATIVE_DIR` 可指向上述已经构建的目录；不设置时由 build.rs 在 Cargo OUT_DIR 单独构建，禁止在构建时下载模型或拉取上游代码。

静态链接：air_llama、llama-common、llama-common-base、llama、ggml、ggml-cpu、ggml-base、cpp-httplib；平台系统库由 build.rs 选择。`cpp-httplib` 是固定上游 common 库的编译依赖；T04 HTTP使用Rust Axum/Hyper，Nexa仍不自动下载模型。

## 所有权与目前限制

- 35bfd85基线shim build_info为2；W02开放模型增量行为identity升3，C ABI布局仍v2，保留air_generate/air_generate_observed；公共protocol1、私有IPC2。实际build_info、worker握手与包身份必须匹配，旧archive不兼容；见`native/llama-shim/include/air_llama.h`，合法句柄/线程前置条件保持，新WindowsCI待验
- 每个进程最多一个引擎、一个模型；prepared 消费一次，生成结束清空请求 KV；取消标志可由另一线程设置
- 原生回调是同步借用，允许decode步骤间共享256KiB预算内可取消、有时限等待；T02已实现4KiB分片、10秒慢消费者和三类deadline。T03已实现独立worker、单一信用账本与消费lease，固定Windows Job/强杀验收通过；详见ADR0004和T03报告
- 35bfd85及旧交付基线仅接入Qwen3/旧模板范围；[ADR0015](decisions/0015-open-model-loading-and-validation-evidence.md)开放模型增量源码已冻结、本地回归通过，由锁定loader判断架构，原始模板需符合窄文本continuation契约。工具链/llama锁不变，矩阵保存精确验证证据，不再作为型号/hash许可名单；新行为待独立验收
- seed `UINT32_MAX` 沿用锁定上游的随机哨兵，其他 seed 在相同环境尽力复现；不承诺跨设备逐字相同
- 原生GGUF的非ASCII路径由上游`ggml_fopen`转为宽字符；T05 Release CI已用中文/空格源模型及受控导入目录实际加载，T03的worker可执行文件及参数路径验证仍单独留证

## Windows CI

`.github/workflows/native-windows.yml` 在已授权的 `codex/nexa-native-baseline` 分支运行。使用 Windows Server 2022 / Visual Studio 2022 runner，输出上游生成、五次 bench 和原生 suite 报告，状态必须以对应提交的实际 Actions 结果为准。工作流文件存在不表示已经执行或通过。

上游固定合成输入：`tests/fixtures/upstream-prompt-zh.txt`，SHA-256 `7720850391d3ad1d502c58fa078a430a045464757c67feac9b812cdceda82a67`。这是锁定模型原始 Jinja 模板在关闭思考模式下的渲染结果；运行前先核对模型/模板 hash。

首个Windows运行[36738163612](https://github.com/Naza3/Nexa/actions/runs/36738163612)的原生构建及CTest成功；Rust静态链接未继承CMake系统依赖，ggml-cpu读取CPU名称使用的Reg*符号需要显式Advapi32。模型下载/推理阶段均被跳过，不能记为失败推理或通过验收。

上游自动基线执行改由 `python scripts/run_upstream_baseline.py --completion <exe> --bench <exe> --model <gguf> --prompt-file tests/fixtures/upstream-prompt-zh.txt --out-dir artifacts/verification --timeout-seconds 300` 承担；每个进程有明确终态与独立日志，不以整个CI job期限代替原生工具时限。脚本测试命令 `python -m unittest discover -s scripts -p 'test_*.py'`。

诊断阶段测试线程显式受控：CI通过NEXA_TEST_THREADS传递min(4,runner可用逻辑CPU)，Python上游与Rust example/集成测试采用同值；可用--threads作明确对照，超过可用CPU会在报告标识，不静默夹紧。此变更只调整验证配置，不修改生产LoadOptions默认值。上游额外1/4线程短诊断仍保留失败状态并单列diagnostic_result，所选线程的正常基线决定验收退出码；不通过延长总超时掩盖异常。


2026-09-30 23:45 UTC，[固定提交 d3d7cf2 的 Windows CI](https://github.com/Naza3/Nexa/actions/runs/36791679663) 全部通过。选定 2 线程 / 2 逻辑 CPU 配置的上游生成、五次 bench、自有 suite 和真实恢复测试通过；探索性 4 线程配置仍超时。具体统计、模型与 artifact 校验见本轮验证记录；不把 CI 配置推广为任意目标硬件或线程数的性能保证。


2026-10-01 00:37 UTC，T02实现提交`bc316da6a66eb52a24ee7a5cb56d8f8c45d1ad37`的[Windows CI](https://github.com/Naza3/Nexa/actions/runs/36796147278)全部通过：原生/整体Rust检查、固定模型身份、上游与native suite、三项adapter真实回归和store→actor→host真实链路。shim2的mid-prefill观察取消3.4101ms，经actor取消至公共终态15.7851ms（各单次功能测量）。精确artifact完整性与边界见[T02报告](verification/2026-10-01-t02-runtime.md)。


2026-10-01 01:47:46 UTC，T03实现`a8930494f62909cccb11876011a650d9713bc74c`的[Windows CI36801681068](https://github.com/Naza3/Nexa/actions/runs/36801681068)全部通过。新增纯Rust process-host/runtime-ipc及native runtime-worker；Windows FFI使用锁定windows-sys0.61.2。在native构建前以不存在AIR_NATIVE_DIR独立构建父端，实际Job/父退出/强杀、信用和真实双进程链路通过；仅固定Server2022/2逻辑CPU/2推理线程/context2048组合，4线程超配诊断仍60秒超时。ZIP摘要、测试数量、CRLF锁文件摘要等精确证据见[T03报告](verification/2026-10-01-t03-worker.md)。


## T04 HTTP 依赖锁（固定 Windows CI 已通过）

新增依赖已实际在Linux和Windows构建/运行；T04实现ccb2053的Windows CI36816604494通过，具体结果见T04记录。Cargo.lock固定：Axum0.8.9、Tokio1.53.1、Hyper1.11.1、hyper-util0.1.21、bytes1.12.1、http-body-util0.1.5、tower0.5.3、hmac0.12.1、subtle2.6.1、getrandom0.4.3、toml0.9.12+spec-1.1.0；Windows直接FFI使用windows-sys0.61.2（旧间接依赖另保留0.59.0）。Hyper在workspace manifest也精确锁1.11.1，Queue/writev与Bytes owner源码假设不能无验证升级。

T04基线LF Cargo.lock SHA-256为`5df74f8dae12b0e546551fa20e087e9c0595eb3cbe6811fb2f9d18f07cfe94da`；Windows checkout换行与artifact摘要另记，不混为同一字节文件。管理CLI/API正常依赖不含engine-host/llama-adapter/runtime-worker；实际缺失native目录的独立构建结果见[T04记录](verification/2026-10-01-t04-http-cli.md)。


## T05 Windows Release 便携构建（固定 Server2022 CI 与Win10短验通过，A20待验）

T05保留同一锁定源码/CPU范围和既有`build/native-release`，不并排重做另一套native workflow。CMake显式VS2022/x64/Release/`MultiThreadedDLL`（/MD），关闭`GGML_BACKEND_DL`及CUDA/Vulkan/Metal/OpenMP/隐式native优化。每个配置生成`air-native-Release.txt`，验证精确source/build/架构/CRT/选项/archive路径；原始绝对身份只留build，不进入分发manifest。Rust首次缺失native目录独立构建CLI，worker再链接该已核验Release树。

`cargo run --locked -p xtask -- build --platform windows-x64 --backend cpu`使用`build/windows-x64-cpu/cargo/x86_64-pc-windows-msvc/release`构建两个产品及独立验收器。MSVC/SDK具体版本和实际VS edition来自本次构建manifest，不能把早期19.44/SDK10.0.26100.0硬套到新runner。产品与验收器各自解析普通/延迟PE依赖并补齐既有VS所提供的未修改Release x64 CRT。额外运行库安装、新协议接受或不一致源需另行报告，不能从System32搬DLL或静默更换工具链。

manifest/SHA256SUMS、许可原文/清单、ZIP/hash分别核对。包内不包含模型、数据、测试凭据、PDB或验收程序；验收器为独立ZIP且自带所需CRT。CI要求产品与工具manifest均为GITHUB_SHA且project_dirty=false；解压后重新校验identity与字节hash。精确交付与法律边界见[ADR0006](decisions/0006-t05-windows-portable-package.md)，结果见[T05报告](verification/2026-10-01-t05-windows-package.md)。

T05源码6a7e9d0的Cargo.lock LF SHA-256为`4c7533fa5c496faafc6c74bf4b222120d6dd7331dcfe80ec230337e02a8ecf20`；Windows checkout的CRLF hash为`826a1be2f952d934a85c525c240780f0223755f8390e64e58da9fa506f2883cb`，已从同一LF源换行转换精确复算。以上T04的锁hash只指其历史基线。CI证据通过`scripts/stage_ci_evidence.py`闭合允许列表输出，保留模型/模板/fixture/工具/失败身份，不再上传任意artifacts目录正文。

第五轮[CI36829233039](https://github.com/Naza3/Nexa/actions/runs/36829233039)实际成功组合：Server2022 build10.0.20348.5622 / image20260927.320.1 / AMD EPYC7763 / 2逻辑CPU；VS2022 Enterprise17.14.37710.0、MSVC19.44.35229.0、VCTools14.44.35207、SDK10.0.26100.0。CRT来自所选实例Redist14.44.35112，DLL文件版14.44.35211.0，产品3个/工具1个app-local DLL均由固定系统PowerShell5.1验证Valid/Microsoft。产品/工具源manifest均6a7e9d0、tree498d0a9、dirty=false；详细hash/体积/真实Release验收见T05报告。此CI安装环境不代替Windows 10无开发工具/离线验收。


## T06 桌面依赖与构建隔离（新目录版CI/产物复核通过，原生UI待验）

实际registry精确锁与本地验证组合：Node24.19.0/npm11.9.0，React/ReactDOM19.3.0、Tauri JS API/CLI2.12.1、Vite8.3.1、TypeScript5.9.3、Vitest5.0.3、ESLint10.11.0。`apps/desktop/package-lock.json`独立管理前端；`npm ci`、typecheck/lint、36tests、生产build与audit0已实际通过。生产JS260464bytes、CSS17539bytes、HTML457bytes；构建排除了显式preview mock，未验证的浏览器交互和原生UI仍分层保留。

`apps/desktop/src-tauri/Cargo.lock`是独立workspace锁，通过path依赖纯Rust bridge；与根锁分开`--locked`，不链接/覆盖根锁。当前图精确tauri2.12.1、tauri-build2.7.1、rfd0.17.2、clipboard-win5.4.1、serde1.0.229、serde_json1.0.151、uuid1.26.1、sha2 0.10.9。对比根锁同名依赖，根中版本均保留于桌面图；Tauri另引入并存toml1.1.6、syn2等，不改既有runtime的版本。Windowsnormal图没有engine-host/llama-adapter/runtime-worker。

Linux壳使用专用target完成5共享模块测试、clippy/fmt；Windows-target依赖/ACL已解析生成，但真实cross build在`llvm-rc`缺失处退出101，不能称Windows壳构建已过。真正MSVC目标构建通过新增Windows CI执行：`build/desktop/cargo/x86_64-pc-windows-msvc/release/nexa-desktop.exe`，Tauri `--no-bundle`后由桌面packager产出ZIP，不自动安装WebView2。Tauri build-script按`CARGO_CFG_TARGET_OS`运行，精确AppManifest能力不因Linux宿主而跳过。

桌面原许可库存含registry省略原文的精确补件，以及已核对的Microsoft WebView2 SDK1.0.3800.47静态x64loader原LICENSE/NOTICE。SDK整包SHA256为`56c9f26bdd07916a2d1949fb58a5c7e434dfa1173577dca879206050c4e718db`，registry crate中的loader与SDK原件逐字节一致，SHA256为`89c6b872783b8f6c3cedbff618adb42082d455c615453ae10cfc753f1e8f25d8`。来源/许可校验清单见`packaging/desktop-windows/third-party/`，打包时再次按hash/源revision验证，不在打包时联网下载补件。

锁文件实际字节hash写入每次桌面manifest，Windows checkout换行差异不以Linux字节hash硬断言。完整来源一致性必须与同次T05 runtime manifest匹配，不能拿另一源码版本的runtime目录拼包。首轮CI36838790221真实Windows编译/PE/许可打包已过，因Tauri CLI对manifest注入空features导致dirty，T05/T06解压验收均被洁净源码门槛拦下。修复将本版CLI确切生成的features数组纳入源码，并在Tauri后早期断言clean，保持后续门槛；第二轮CI36844727718实际通过上述洁净检查、T05解压真实验收和桌面诊断，探测已装WebView2为131.0.2903.86。后续真实bridge验收返回exit1，尚不能交付已验收桌面包。具体结果见[T06验证](verification/2026-10-01-t06-desktop.md)。


Tauri CLI2.12.1会在执行cargo前规范化依赖features。`tauri = { package = "tauri", version = "=2.12.1", features = [] }`与`tauri-build = { version = "=2.7.1", features = [] }`是本次实际CLI生成并二次运行保持字节不变的形式；没有改变依赖版本或根锁。源码清洁度断言属于必要发行门槛，不能因工具写回而忽略manifest。


第五轮源码c8dff8的CI36855675437保留同一工具链与依赖锁，真正Tauri Release/源码clean/桌面诊断和T05完整解压验收通过；最终桌面bridge步骤被15分钟平台时限终止，未完成产品验收。当前修复仅涉及harness私有文件报告及Python直接子进程有界等待，不升级依赖、不改变生产启动flags或延长时限；完整证据与未通过项见T06记录。


第六轮源码`bc43e0f3ac215d41e5d93cccf670ab43d67d41c0`的[CI36864041027](https://github.com/Naza3/Nexa/actions/runs/36864041027)于13:31:29 UTC已确认completed/success，依赖/工具链锁保持原组合。真正Tauri Release、早期Windows无模型传输回归、T05与T06完整解压/真实模型bridge均通过；桌面ZIP9,507,755bytes（SHA256 `2ea95591ddabc4e7ae930ea166e6343f6aad275fa01975d3eb7bebbef0546fe9`），manifest SHA256 `c8a259d2ba20fa90c5dac7e21d8e13e7c22fd28e091d11c7b037ecb2cdc34673`，完整下载包的库存/hash/许可/PE闭包已独立复核。该CI未驱动原生窗口；另有独立手工验收确认旧包核心UI与两种关闭通过，新目录选择/零复制/自动名称另需构建与实测，详见T06记录。

外部目录扩展的独立Tauri锁仅新增desktop-bridge→model-store、nexa-desktop→已存在的windows-sys 0.61.2两条依赖边；后者用于只读GetDriveTypeW本地盘类型检查，不升级registry版本或更换框架。新目录版Windows CI及产物独立复核已通过，原生目录UI仍待验；依赖图保持不含native推理crate。

2026-10-02，目录/诊断功能源码`75e458f60cbbfc2b136d8396d7e824c3fc07f23e`的[Windows CI36948947690](https://github.com/Naza3/Nexa/actions/runs/36948947690)已成功，native job`110657335010`含真实模型/runtime/HTTP/CLI、桌面包构建和解压后bridge验收通过。下载产物独立完整性复核已通过并交付：原ZIP9,785,013bytes，SHA256 `1d4f89eeb9c03b14aecaa7199c847413ee85b215cd44ee8bf9436bbb14596858`，source tree `8cec0b3be1d8c4f3442d72f2c27f87f8506d9523`、dirty=false。实际750文件、两层manifest/hash、6个PE/依赖闭包及许可均核验，根/壳锁按该提交Windows CRLF字节复核；原始构建工作树未独立重建。新目录原生UI仍未测，不将CI/包核验等同完整桌面验收，精确结果见[T06记录](verification/2026-10-01-t06-desktop.md#第七轮目录版windows-ci成功独立复核与交付2026-10-02)。本轮范围收敛不改变该提交的工具链。



## 当前Windows范围与后续门槛

- 最新389eeef未升级上述llama/工具链；[最终CI/交付记录](verification/2026-10-02-windows-model-compatibility.md)覆盖模型兼容性增量
- Windows10 x64/i5-8400是首要目标，其他Intel/AMD桌面CPU和Windows11按实际指令集/设备扩大；SSE4.2/AVX/AVX2/F16C/FMA/BMI2要求不能由GGML_NATIVE=OFF自动消除
- W02新增模型、模板/工具能力或引擎升级分别锁定并回归；W04独立记录dsh/pi-ai精确依赖与真实出站fixture，不影响现有运行时依赖锁
- 旧移动工具链/SDK和许可研究见[原构建锁快照](archive/windows-focus-2026-10-03/docs/build-lock.md)与[历史索引](archive/windows-focus-2026-10-03/INDEX.md)；源码/隔离CI不改
