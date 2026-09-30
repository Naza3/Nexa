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

退出0；返回非空中文且无思考标签。原始输出/统计留在 `artifacts/verification/upstream-zh.*`。该次运行与开发编译共享机器，只有功能意义，不据此声明吞吐门槛、10%包装开销或硬件性能。后续有界执行器在Linux真实复验：completion退出0、5.66s；bench退出0、23.70s，含默认warmup及两个测试各5次采样。pp128中位67.262 tokens/s（56.2325–112.938），tg32中位15.5463 tokens/s（9.1496–17.4001）；共享Xeon开发机波动明显，不作目标设备或包装开销承诺。原始JSON在artifacts/verification/bounded-linux/。

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
- 100短请求/20加载卸载的长期内存曲线、峰值/释放内存和TTFT均unavailable；仅Linux上游bench的5次统计已有独立报告
- T00/T01保持待验证，T02–T10和S00–S04未开始。目标平台、API、UI、摘要质量不由fake或仅构建替代

最终native报告：`artifacts/verification/linux-native-smoke.json`，SHA-256 `58c6a8fa3a670e2c0d29ec16d764e5462fd92b02f6d6f610daadf1793ccd4c52`。长输入的模板后prompt为821 tokens，输出84 tokens自然stop；这只是推理功能测试，不是Telegram业务质量评分。

最终集成日志：`artifacts/verification/linux-integrated-checks.log`，SHA-256 `8b115385099f8d962d3c6692d13afa5366ef15e03921a8690251cef49f8cf765`。最终基线身份报告SHA-256 `fd458c809a3cb691fac0a2301fdd7f6a7211d5ada9acb818b3c844c5beb30add`。

A02后续证据：`/tmp/nexa-a02-real-model.log` 和 `/tmp/nexa-a02-full-real-model.log`。这是本轮第一提交之后追加的测试，不能追溯记为第一提交Windows CI已经覆盖。

## 首次Windows CI结果

提交 `79f5362821080add23a5359620911e77a4c85d42`，运行 [36738163612](https://github.com/Naza3/Nexa/actions/runs/36738163612)：原生Release（shim、completion、bench）构建和CTest 1/1通过；Rust fmt通过，cargo test在链接处失败，错误为ggml-cpu依赖的RegCloseKey/RegOpenKeyExA/RegQueryValueExA未解析。Rust build.rs需要显式链接Windows系统Advapi32。模型下载和全部Windows真实推理尚未执行。首跑没有生成业务验证artifact，完整失败证据保留于Actions job日志；后续workflow已增加构建/测试日志归档。

A02多轮测试与链接修复需要新提交的Windows CI，不能用Linux补测抹去此次失败或追溯声称覆盖。

## 第二次Windows CI与有界诊断

提交 `1d26ae89365472612e57c674df57c04767f42a40` 的 [运行36740216058](https://github.com/Naza3/Nexa/actions/runs/36740216058)已经通过原生构建、Rust fmt/test/clippy和真实模型身份核对。上游步骤自16:01:22 UTC运行，截至16:13仍未结束；运行中的job日志接口返回404 BlobNotFound，尚不能确定completion还是bench耗时，也不能断言控制台交互就是根因。

修复使用 `scripts/run_upstream_baseline.py`：每个上游进程分别300秒期限，stdin直接EOF，completion明确simple-io，stdout/stderr直接落文件，保存退出码/超时/耗时与SHA。超时自动kill并reap；未运行bench明确skipped。外层step12分钟，保留always-upload收集证据的余量。不绕过非思考、真实中文或5次benchmark采样检查。4项执行器测试通过，真实Linuxcompletion/bench均通过；Windows行为等待新提交重验。

阶段判断：Windows真实上游固定模型基线尚未建立，所以T00不能完成。T01已经有Linux真实流式、停止、模板/预算和重复加载证据；Windowsnative suite与独立prefill取消观测仍待补。Android真机、独立无开发工具Windows机、100请求/20加载长期趋势与全性能矩阵属于T05/T07/T09，不能把它们误当作开始T02的全部前提；也不把这些缺项写成通过。下一有界步骤是结束Windows基线和native suite，补最小prefill取消观测，再依路线评估T02。

第三次Windows运行 [36743342648](https://github.com/Naza3/Nexa/actions/runs/36743342648) 再次通过原生/Rust/模型hash，但执行器测试在Windows默认cp1252写中文时暴露未指定编码（UnicodeEncodeError）。上游进程未启动。修复为测试全部文本读写显式UTF-8，并启用EncodingWarning-as-error回归；纯脚本测试前移工具准备阶段，以尽早发现平台脚本错误，不跳过验证。

## 第四次Windows运行：明确warmup异常，启动线程对照

[运行36745129865](https://github.com/Naza3/Nexa/actions/runs/36745129865) 的工具/脚本测试、原生/Rust构建与模型身份均通过；completion在300.344秒被有界执行器终止，bench未执行。原始stderr记录threadpool于0.815s初始化，warmup结束/进入generate已到152.690s，系统报告4推理线程但仅2逻辑CPU；尚无文本输出。该证据支持线程超配是候选原因，不能据此认定唯一根因或宣布修复成功。

artifact `11112772308` 的ZIP SHA-256为 `f50d169ca97890ad356ad38270b2d527b944eb28b5a7bc67a5a3d2a10d0fd0d4`；已核对后解压到本机 `artifacts/verification/windows-run4/`。第二跑取消后的artifact `11111716061` 则包含少量中文并显示Interrupted by user，bench不存在，因此旧步骤也确定停留在completion。

下一次有界对照：同一模型、模板、参数，仅改变推理线程；上游1/4线程短生成各60秒，正常生成/5次bench和自有native suite统一使用min(4,runner可用CPU)。探索性诊断逐项保留原始失败/超时并单列diagnostic_result；仅所选线程配置的正常生成/bench决定baseline_result和上游步骤退出码，不把4线程超配探针当成2线程配置的发布要求，也不宣称4线程已支持；模型身份通过后自有suite/恢复测试独立运行，整体CI保留失败，便于区分上游工具与核心。生产LoadOptions默认值不因测试环境改动。所有实际线程/可用CPU/超配状态进入报告。

同时保存本次编译工具/静态库及逐文件SHA清单到同仓库artifact；任何后续复用都必须先核对artifact及清单hash与来源提交，不将不同源码的旧二进制冒充当前构建。

线程配置修复的Linux本地复验：真实Rust生命周期/A02分别在1线程和2线程完整通过（40.41s / 56.77s）；2线程xtask九场景/十轮全通过，报告 `artifacts/verification/linux-native-smoke-threads2.json`，SHA-256 `b89ddd7ac17e63aa25201b197901639662d7d53df08eab55aaed6abc414624d5`，每轮实际threads=2、available_parallelism=9、oversubscribed=false。上游1/4线程短对照分别3.13s / 5.74s，正常2线程生成3.43s、5次bench45.68s，均退出0；这些是9逻辑CPU的Linux开发环境结果，不证明Windows2逻辑CPU已恢复。

执行器严格编码及判定测试共6项：探索性诊断失败仍逐项保留；正常基线失败必定非零；正常基线成功不被额外超配探针误标为发布失败。所有诊断均在报告单列，不扩大4线程支持声明。Windows新结果仍待CI产生。


## 第五次 Windows 运行：T00/T01 最小阶段门槛通过

提交 `d3d7cf2d9f0d2ce7aa03ca7d27787a2f423f3144` 的 [运行 36791679663](https://github.com/Naza3/Nexa/actions/runs/36791679663) 于 2026-09-30 23:45 UTC 全部成功。原生 Release/CTest、Rust fmt/test/clippy、脚本编码回归、真实模型/模板身份校验及推理检查均通过。设备为 Windows Server 2022 10.0.20348.5622、AMD EPYC 7763 runner，实际仅 2 逻辑 CPU。

- 1 线程短探针：16-token 上限，退出 0，2.187 秒
- 4 线程超配短探针：60.406 秒超时，stdout 为空；保留 `diagnostic_result=failed`，不能宣称此组合通过
- 选定 2 线程正常中文生成：退出 0，2.156 秒；有真实中文、无思考标签
- 选定 2 线程上游 bench：退出 0，17.313 秒；pp128 和 tg32 各 5 个样本。中位分别 68.1968 和 32.6147 tokens/s，只代表此 CI 配置，不是包装开销或目标 PC 保证
- 自有 native suite：九场景、十轮均通过，实际线程为 2 且无超配；中英文、长输入、精确预算、三类预取消、生成中跨线程取消、消费者停止和重复加载均通过
- 显式真实集成测试：2 项通过，14.82 秒；stop/取消/panic 后恢复、context 33 逻辑边界与 A02 system/历史/请求隔离均覆盖。日志中的 intentional consumer panic 是被捕获的预期测试路径，测试最终成功

同一 runner 的 1/2 线程成功与 4 线程超时加强了超配与异常的关联；没有改变 llama commit。保留配置限制，不声明所有 Windows CPU/线程组合均可用，也不把生产默认参数视为已调整。

验证 artifact `11132296863`，ZIP SHA-256 `4ca62c5f4ddc32492a915df69c3e4b79ab91f1cf81f1be2e87dc1e9628b8d74d`，已下载核对并安全解压至 `artifacts/verification/windows-run5/`。原始 `upstream-processes.json` 分别保存正常基线与探索诊断状态；native JSON 中平台验收字段是自动工具的保守占位，阶段结论在此按实际设备证据人工确认。编译工具 artifact `11132291996`，ZIP SHA-256 `4fad0b2fb4a9ee666cd71de722ae69bb2ff116b162df040744e8e9f9363083d7`，本轮仅核对远程 metadata，未下载或复用。

T00 已满足固定 Windows CPU 上游真实输入/统计门槛；T01 已满足真实中英文流式、模板/特殊 token、重复加载释放和取消的最小交付。允许进入 T02。A08 的独立 prefill 中途取消耗时仍未测，须在 T02 A05–A12 集成验收补齐；100 请求/20 加载长期内存趋势、完整性能、无开发工具 Windows 发行与 Android 真机仍属于后续任务。以上结论更新此前各历史小节的“待验证”状态，不抹去旧失败。
