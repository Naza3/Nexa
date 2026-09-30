# T00/T01 原生开发基线验证

日期：2026-09-30；平台：Linux x86_64 开发容器。原始提交 `0d3a3cea32b813dad0857f9e1a1e41862ce27168`，本轮工作树尚待父任务提交。固定依赖/设备见 [构建锁](../build-lock.md)，模型见 [矩阵](../model-matrix.md)。本记录不把 Linux 结果标为 Windows 或 Android 验收。

## 修改范围

- 最小 workspace、Rust/Cargo 锁、llama.cpp 固定 submodule、忽略规则
- `native/llama-shim`：C ABI、单线程对象生命周期、仅原子取消跨线程、异常边界、真实模板/分词/推理、UTF-8/stop缓冲
- `runtime-types`：本轮实际DTO/校验；`llama-adapter`：安全所有权封装、回调panic隔离、真实模型example与集成测试
- `xtask`：GGUF/依赖基线验证、真实native suite、有限输出/超时、失败退出与可审查JSON证据
- Windows CI及当前文档；未创建后续阶段空实现

## 执行结果

下列命令均在项目根目录运行；模型参数指向已实际下载并hash核对的官方文件。原生目录 `build/native-release`，Rust验证使用独立 `CARGO_TARGET_DIR=/tmp/nexa-rust-target` 和 `AIR_NATIVE_DIR=$PWD/build/native-release`。日志只保留合成输入，不包含用户聊天或凭据。

| 实际检查 | 结果 / 退出码 | 证明范围 |
| --- | --- | --- |
| `cmake -S native/llama-shim -B build/native-release -G Ninja -DCMAKE_BUILD_TYPE=Release` | pass / 0 | 固定Linux CPU配置 |
| `cmake --build build/native-release --target air_llama llama-completion llama-bench -j 4` | pass / 0 | 自有shim与上游真实工具构建 |
| `cmake --build build/native-release --target air_llama air-stream-test -j 4`；`ctest --test-dir build/native-release --output-on-failure` | pass / 0，1项 | Release中仍生效的显式require检查，非NDEBUG空测 |
| `g++ -std=c++17 -Wall -Wextra -Werror native/llama-shim/tests/stream_buffer_test.cpp ...` | pass / 0 | UTF8全部字节切点、跨chunk stop、4KiB、非法序列拒绝 |
| 同测试 `-fsanitize=address,undefined`，`ASAN_OPTIONS=detect_leaks=0` | pass / 0 | 仅纯流缓冲；不是完整llama内存安全验收 |
| Rust两crate单测与doctest | pass / 0，13单测+7 compile-fail | 参数、取消、回调、线程/借用约束 |
| `cargo test --locked -p llama-adapter --test real_model -- --ignored --test-threads=1`，设置`NEXA_TEST_MODEL` | pass / 0，1项复合真实测试，11.63s | stop首段不泄漏、KV隔离、取消/panic后复用、释放重载、33-token逻辑预算、非思考检查 |
| `cargo build --locked -p llama-adapter --example native-smoke` | pass / 0 | 真实模型验证入口 |
| `cargo run --locked -p xtask -- baseline-verify --model ... --out ...` | pass / 0 | 实际文件/模板hash、架构、量化、工具链、上游源码清洁 |
| 最终九场景 `xtask native-smoke` | pass / 0，9场景、10轮；每轮binary_unchanged=true | 中英文、长输入、预算、加载释放及取消功能；不替代完整A矩阵 |
| `cargo fmt --all -- --check`；`cargo test --locked --workspace`；`cargo clippy --locked --workspace --all-targets -- -D warnings` | pass / 0，25单测+7 compile-fail；真实测试在普通workspace运行中明确ignored，已另行显式执行通过 | 最终集成检查，另含CTest与git diff --check均退出0 |
| Windows workflow YAML解析 | pass / 0 | 语法检查，不证明Actions执行 |
| Windows/Android设备验收 | unavailable | 本轮没有目标设备 |

真实Rust测试日志：`/tmp/nexa-rust-verification.log`，SHA-256 `99a2b6514da6e61d3b395956975a99f9e2f068e5ca07c6395a43d136e145c36c`。最终验证example SHA-256 `350c703e01d7ba9d0755313eb168e68fe4c550a5936f731dbf3ff22ab4b79e06`。日志和二进制为本机证据，不提交构建产物。

## 上游真实生成

从模型自身模板以 `enable_thinking=false` 渲染固定中文prompt，保存于 `tests/fixtures/upstream-prompt-zh.txt`。实际执行：

```sh
build/native-release/bin/llama-completion -m "$MODEL" -c 2048 -b 128 -t 4 -n 64 --temp 0 --seed 42 --no-conversation --no-display-prompt -f tests/fixtures/upstream-prompt-zh.txt
```

退出0；返回非空中文且无思考标签。原始输出/统计留在 `artifacts/verification/upstream-zh.*`。该次运行与开发编译共享机器，只有功能意义，不据此声明吞吐门槛、10%包装开销或硬件性能。五次bench尚未本地执行；已列入Windows CI，结果待实际产生。

## 发现并修复的问题

1. 工程为空且HTTPS clone需认证：改用已授权GitHub连接器物化文件；原始blob/tree/commit SHA逐层核对，保留原提交祖先
2. 上游当前`llama-cli`目标受server开关约束：使用实际构建的`llama-completion`；未改变锁定commit
3. 上游completion初始模板分支忽略reasoning参数：固定原模板渲染基线，自有shim直接传关闭标志；未对输出做字符串删除
4. sampler_sample已经accept：去掉重复accept；usage在EOG判断前计入实际采样终止token
5. 上游KV按256取整：保存用户逻辑context限制，真实33-token测试证明不静默扩大预算
6. LeakSanitizer在沙箱ptrace下不可运行：如实保留限制，仅ASan/UBSan纯缓冲检查通过

## 验收映射与未验项

- A01/A04/A05/A08/A09/A19仅部分原生语义有证据，不称HTTP/平台完整通过
- A02已新增Linux真实多轮专项：末问题29 tokens、历史75 tokens、首条system+历史113 tokens；单轮↔多轮在同模型上下文交替后greedy输出与usage保持运行内一致。定向12.97s，连同原生命周期测试显式执行两项均通过（34.37s）。Windows对应新增用例仍待包含该测试的新提交CI；A03 SSE尚未实现；prefill中途取消的独立计时未测
- 100短请求/20加载卸载的长期内存曲线、峰值/释放内存、TTFT和5次性能中位数均unavailable
- T00/T01保持待验证，T02–T10和S00–S04未开始。目标平台、API、UI、摘要质量不由fake或仅构建替代

最终native报告：`artifacts/verification/linux-native-smoke.json`，SHA-256 `58c6a8fa3a670e2c0d29ec16d764e5462fd92b02f6d6f610daadf1793ccd4c52`。长输入的模板后prompt为821 tokens，输出84 tokens自然stop；这只是推理功能测试，不是Telegram业务质量评分。

最终集成日志：`artifacts/verification/linux-integrated-checks.log`，SHA-256 `8b115385099f8d962d3c6692d13afa5366ef15e03921a8690251cef49f8cf765`。最终基线身份报告SHA-256 `fd458c809a3cb691fac0a2301fdd7f6a7211d5ada9acb818b3c844c5beb30add`。

A02后续证据：`/tmp/nexa-a02-real-model.log` 和 `/tmp/nexa-a02-full-real-model.log`。这是本轮第一提交之后追加的测试，不能追溯记为第一提交Windows CI已经覆盖。

## 首次Windows CI结果

提交 `79f5362821080add23a5359620911e77a4c85d42`，运行 [36738163612](https://github.com/Naza3/Nexa/actions/runs/36738163612)：原生Release（shim、completion、bench）构建和CTest 1/1通过；Rust fmt通过，cargo test在链接处失败，错误为ggml-cpu依赖的RegCloseKey/RegOpenKeyExA/RegQueryValueExA未解析。Rust build.rs需要显式链接Windows系统Advapi32。模型下载和全部Windows真实推理尚未执行。首跑没有生成业务验证artifact，完整失败证据保留于Actions job日志；后续workflow已增加构建/测试日志归档。

A02多轮测试与链接修复需要新提交的Windows CI，不能用Linux补测抹去此次失败或追溯声称覆盖。
