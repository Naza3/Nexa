# 开发与 Windows 包构建锁

日期：2026-10-01。此文件记录Linux开发与固定Windows CI构建组合；精确支持范围见模型矩阵和验证报告。T00/T01固定Windows 2线程组合已通过，目标i5-8400与Android真机均未验收。

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
| Android NDK/Flutter | unavailable；尚未加入工程依赖 |

没有升级跟随 master 的构建脚本。CMake 检查 submodule HEAD，不匹配直接失败。原生模型默认日志被抑制，以避免上游输出完整用户路径；开发基线用合成输入，原始上游日志只留在忽略目录。

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

- shim build_info版本2，保留air_generate兼容入口并增加air_generate_observed；见 `native/llama-shim/include/air_llama.h`；调用方必须遵守合法句柄和线程归属前置条件
- 每个进程最多一个引擎、一个模型；prepared 消费一次，生成结束清空请求 KV；取消标志可由另一线程设置
- 原生回调是同步借用，允许decode步骤间共享256KiB预算内可取消、有时限等待；T02已实现4KiB分片、10秒慢消费者和三类deadline。T03已实现独立worker、单一信用账本与消费lease，固定Windows Job/强杀验收通过；详见ADR0004和T03报告
- 目前仅接入 Qwen3 架构及能明确关闭思考的模板；验收支持以精确模型矩阵为准
- seed `UINT32_MAX` 沿用锁定上游的随机哨兵，其他 seed 在相同环境尽力复现；不承诺跨设备逐字相同
- 原生GGUF的非ASCII路径由上游`ggml_fopen`转为宽字符；该模型路径场景尚未单独Windows实测。T03已实际验证worker可执行文件及参数的空格/Unicode路径，二者不混同

## Windows CI

`.github/workflows/native-windows.yml` 在已授权的 `codex/nexa-native-baseline` 分支运行。使用 Windows Server 2022 / Visual Studio 2022 runner，输出上游生成、五次 bench 和原生 suite 报告，状态必须以对应提交的实际 Actions 结果为准。工作流文件存在不表示已经执行或通过。

上游固定合成输入：`tests/fixtures/upstream-prompt-zh.txt`，SHA-256 `7720850391d3ad1d502c58fa078a430a045464757c67feac9b812cdceda82a67`。这是锁定模型原始 Jinja 模板在关闭思考模式下的渲染结果；运行前先核对模型/模板 hash。

首个Windows运行[36738163612](https://github.com/Naza3/Nexa/actions/runs/36738163612)的原生构建及CTest成功；Rust静态链接未继承CMake系统依赖，ggml-cpu读取CPU名称使用的Reg*符号需要显式Advapi32。模型下载/推理阶段均被跳过，不能记为失败推理或通过验收。

上游自动基线执行改由 `python scripts/run_upstream_baseline.py --completion <exe> --bench <exe> --model <gguf> --prompt-file tests/fixtures/upstream-prompt-zh.txt --out-dir artifacts/verification --timeout-seconds 300` 承担；每个进程有明确终态与独立日志，不以整个CI job期限代替原生工具时限。脚本测试命令 `python -m unittest discover -s scripts -p 'test_*.py'`。

诊断阶段测试线程显式受控：CI通过NEXA_TEST_THREADS传递min(4,runner可用逻辑CPU)，Python上游与Rust example/集成测试采用同值；可用--threads作明确对照，超过可用CPU会在报告标识，不静默夹紧。此变更只调整验证配置，不修改生产LoadOptions默认值。上游额外1/4线程短诊断仍保留失败状态并单列diagnostic_result，所选线程的正常基线决定验收退出码；不通过延长总超时掩盖异常。


2026-09-30 23:45 UTC，[固定提交 d3d7cf2 的 Windows CI](https://github.com/Naza3/Nexa/actions/runs/36791679663) 全部通过。选定 2 线程 / 2 逻辑 CPU 配置的上游生成、五次 bench、自有 suite 和真实恢复测试通过；探索性 4 线程配置仍超时。具体统计、模型与 artifact 校验见本轮验证记录；不把 CI 配置推广为 i5-8400 或任意线程数性能保证。


2026-10-01 00:37 UTC，T02实现提交`bc316da6a66eb52a24ee7a5cb56d8f8c45d1ad37`的[Windows CI](https://github.com/Naza3/Nexa/actions/runs/36796147278)全部通过：原生/整体Rust检查、固定模型身份、上游与native suite、三项adapter真实回归和store→actor→host真实链路。shim2的mid-prefill观察取消3.4101ms，经actor取消至公共终态15.7851ms（各单次功能测量）。精确artifact完整性与边界见[T02报告](verification/2026-10-01-t02-runtime.md)。


2026-10-01 01:47:46 UTC，T03实现`a8930494f62909cccb11876011a650d9713bc74c`的[Windows CI36801681068](https://github.com/Naza3/Nexa/actions/runs/36801681068)全部通过。新增纯Rust process-host/runtime-ipc及native runtime-worker；Windows FFI使用锁定windows-sys0.61.2。在native构建前以不存在AIR_NATIVE_DIR独立构建父端，实际Job/父退出/强杀、信用和真实双进程链路通过；仅固定Server2022/2逻辑CPU/2推理线程/context2048组合，4线程超配诊断仍60秒超时。ZIP摘要、测试数量、CRLF锁文件摘要等精确证据见[T03报告](verification/2026-10-01-t03-worker.md)。


## T04 HTTP 依赖锁（固定 Windows CI 已通过）

新增依赖已实际在Linux和Windows构建/运行；T04实现ccb2053的Windows CI36816604494通过，具体结果见T04记录。Cargo.lock固定：Axum0.8.9、Tokio1.53.1、Hyper1.11.1、hyper-util0.1.21、bytes1.12.1、http-body-util0.1.5、tower0.5.3、hmac0.12.1、subtle2.6.1、getrandom0.4.3、toml0.9.12+spec-1.1.0；Windows直接FFI使用windows-sys0.61.2（旧间接依赖另保留0.59.0）。Hyper在workspace manifest也精确锁1.11.1，Queue/writev与Bytes owner源码假设不能无验证升级。

T04基线LF Cargo.lock SHA-256为`5df74f8dae12b0e546551fa20e087e9c0595eb3cbe6811fb2f9d18f07cfe94da`；Windows checkout换行与artifact摘要另记，不混为同一字节文件。管理CLI/API正常依赖不含engine-host/llama-adapter/runtime-worker；实际缺失native目录的独立构建结果见[T04记录](verification/2026-10-01-t04-http-cli.md)。


## T05 Windows Release 便携构建（实际 CI 待验证）

T05保留同一锁定源码/CPU范围和既有`build/native-release`，不并排重做另一套native workflow。CMake显式VS2022/x64/Release/`MultiThreadedDLL`（/MD），关闭`GGML_BACKEND_DL`及CUDA/Vulkan/Metal/OpenMP/隐式native优化。每个配置生成`air-native-Release.txt`，验证精确source/build/架构/CRT/选项/archive路径；原始绝对身份只留build，不进入分发manifest。Rust首次缺失native目录独立构建CLI，worker再链接该已核验Release树。

`cargo run --locked -p xtask -- build --platform windows-x64 --backend cpu`使用`build/windows-x64-cpu/cargo/x86_64-pc-windows-msvc/release`构建两个产品及独立验收器。MSVC/SDK具体版本和实际VS edition来自本次构建manifest，不能把早期19.44/SDK10.0.26100.0硬套到新runner。产品与验收器各自解析普通/延迟PE依赖并补齐既有VS所提供的未修改Release x64 CRT。额外运行库安装、新协议接受或不一致源需另行报告，不能从System32搬DLL或静默更换工具链。

manifest/SHA256SUMS、许可原文/清单、ZIP/hash分别核对。包内不包含模型、数据、测试凭据、PDB或验收程序；验收器为独立ZIP且自带所需CRT。CI要求产品与工具manifest均为GITHUB_SHA且project_dirty=false；解压后重新校验identity与字节hash。精确交付与法律边界见[ADR0006](decisions/0006-t05-windows-portable-package.md)，结果见[T05报告](verification/2026-10-01-t05-windows-package.md)。

T05新增Windows FFI/工具依赖与最终Cargo.lock的精确摘要待父级整合后按实际值记录；以上T04的锁hash只指其历史基线，不表示工作树锁文件未变。CI证据通过`scripts/stage_ci_evidence.py`闭合允许列表输出，保留模型/模板/fixture/工具/失败身份，不再上传任意artifacts目录正文。
