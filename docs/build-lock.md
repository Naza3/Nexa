# T00 开发构建锁

日期：2026-09-30。此文件记录已经在 Linux 云开发机上构建的组合；**不是 Windows / Android 支持声明**。T00 的 Windows CPU 基线门槛仍需 Windows 执行，目标 i5-8400 与 Android 真机均未验收。

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
| Windows 编译器/SDK | MSVC 19.44.35229.0 / SDK 10.0.26100.0；Windows Server 2022 10.0.20348；原生Release与CTest已在首跑通过，Rust整体链接待补Advapi32复验 |
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

静态链接：air_llama、llama-common、llama-common-base、llama、ggml、ggml-cpu、ggml-base、cpp-httplib；平台系统库由 build.rs 选择。`cpp-httplib` 是固定上游 common 库的编译依赖，不表示 Nexa 已实现 HTTP 或模型在线下载。

## 所有权与目前限制

- ABI v1 见 `native/llama-shim/include/air_llama.h`；调用方必须遵守合法句柄和线程归属前置条件
- 每个进程最多一个引擎、一个模型；prepared 消费一次，生成结束清空请求 KV；取消标志可由另一线程设置
- 原生回调是同步借用，必须及时返回；有界异步消费者队列、10 秒慢消费者和执行 deadline 属于后续 T02/T03，不在这里冒充实现
- 目前仅接入 Qwen3 架构及能明确关闭思考的模板；验收支持以精确模型矩阵为准
- seed `UINT32_MAX` 沿用锁定上游的随机哨兵，其他 seed 在相同环境尽力复现；不承诺跨设备逐字相同
- Windows UTF-8 路径由上游 `ggml_fopen` 转为宽字符文件打开；已读源码，尚未 Windows 实测

## Windows CI

`.github/workflows/native-windows.yml` 在已授权的 `codex/nexa-native-baseline` 分支运行。使用 Windows Server 2022 / Visual Studio 2022 runner，输出上游生成、五次 bench 和原生 suite 报告，状态必须以对应提交的实际 Actions 结果为准。工作流文件存在不表示已经执行或通过。

上游固定合成输入：`tests/fixtures/upstream-prompt-zh.txt`，SHA-256 `7720850391d3ad1d502c58fa078a430a045464757c67feac9b812cdceda82a67`。这是锁定模型原始 Jinja 模板在关闭思考模式下的渲染结果；运行前先核对模型/模板 hash。

首个Windows运行[36738163612](https://github.com/Naza3/Nexa/actions/runs/36738163612)的原生构建及CTest成功；Rust静态链接未继承CMake系统依赖，ggml-cpu读取CPU名称使用的Reg*符号需要显式Advapi32。模型下载/推理阶段均被跳过，不能记为失败推理或通过验收。
