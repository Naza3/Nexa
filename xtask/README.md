# 当前可执行验证命令

在项目根目录执行；所有命令须使用 `rust-toolchain.toml` 固定工具链。模型须先按 `tests/fixtures/baseline.json` 所列固定 revision 获取，不提交模型。

```sh
cargo test --locked -p xtask
cargo run --locked -p xtask -- baseline-verify --model /path/to/Qwen3-0.6B-Q8_0.gguf --out artifacts/verification/baseline.json
cargo build --locked -p llama-adapter --example native-smoke
cargo run --locked -p xtask -- native-smoke --model /path/to/Qwen3-0.6B-Q8_0.gguf --bin target/debug/examples/native-smoke --threads 2 --out artifacts/verification/native-smoke-threads2.json
```

Windows 的 example 文件名为 `native-smoke.exe`。原生库构建配置以 `docs/build-lock.md` 为准，不通过 xtask 隐式下载或重建依赖。可用 `--device` 补充设备标签、`--manifest` 选择锁定清单；native-smoke 的 `--timeout-seconds` 为每个独立进程的时限，默认 180 秒。

线程数仅影响测试/诊断工具，生产 `LoadOptions` 默认值不变。优先级为 `--threads N`、`NEXA_TEST_THREADS` 环境变量、`min(4, available_parallelism)`；无法查询并行度时保守取 1。显式值必须在 1–256 内；显式超出可用并行度不会被静默改小，报告会标记 `oversubscribed=true`，可用于受控对照。xtask 始终把解析出的值显式传给 example，并检查它报告的实际线程数一致。每次报告记录 actual threads、available parallelism、是否超配及选择来源。基线身份/模型 hash 不因线程数调整而改变；应使用独立报告路径保留此前线程配置的失败证据，不能以新配置通过覆盖或解释为旧配置已修复。

`baseline-verify` 校验精确 Rust/llama 版本、llama 已跟踪源码无修改、真实 GGUF 文件 hash、架构、量化类型、原始模板 hash 和上下文边界。它仅验证基线身份，不能证明推理成功。

`native-smoke` 先执行相同基线校验，再调用已经构建的真实模型 example，检查中英文/长输入非空输出、预算、重复加载释放及取消/消费者停止路径。每个进程的退出码、时间、二进制/输入/输出摘要和观测数值写入 JSON。对子进程 stdout 做大小限制和独立结构断言；不会凭退出 0 或一条 pass 字段宣布成功。报告不保存原文或完整本地路径。重复加载只验证两轮功能生命周期；不宣称已测内存泄漏或性能。中途 prefill 取消、连续 100 请求、20 次加载卸载以及内存释放量均不在此 smoke 覆盖范围。

退出码：0 表示当前请求的验证范围通过；1 表示已执行的验证失败；2 表示参数/前置文件错误。读取不到文件时不会写一份伪成功报告。默认设备标签和无法测量的性能值为 `unavailable`。目标 Windows/Android 验收单独标记 `skipped`；即便在对应 OS 执行，也不能代替指定设备的完整验收。

执行规格中后续阶段的`check`、`test --suite contract`尚未实现，调用会明确失败。T04已有api-smoke，T05新增Windows专用build及独立验收器，范围和关停副作用见下节；每阶段实际验收分别记录，不能凭一条命令宣布T02–T09全完成。


## T02 存储与调度验证

```sh
cargo test --locked -p runtime-types -p runtime-core -p model-store
NEXA_TEST_MODEL=/path/to/Qwen3-0.6B-Q8_0.gguf NEXA_TEST_THREADS=2 cargo test --locked -p llama-adapter --test real_model -- --ignored --test-threads=1 --nocapture
NEXA_TEST_MODEL=/path/to/Qwen3-0.6B-Q8_0.gguf NEXA_TEST_THREADS=2 cargo test --locked -p engine-host --test real_runtime -- --ignored --test-threads=1 --nocapture
```

先重建shim版本2，`AIR_NATIVE_DIR`指向最新构建。`native-smoke`兼容校验现要求shim_version=2；新air_generate_observed保留v1生成入口。adapter真实测试分别记录至少一批prefill成功后及decode阶段取消；host真实测试把原模型复制到临时model-store并验证调度端到端。逻辑fake不能代替这些真实GGUF命令，Windows/Android状态分别记录。


## T04 实际 HTTP/CLI smoke

`cargo run --locked -p xtask -- api-smoke --base-url http://127.0.0.1:PORT --data-dir TEST_DATA --model qa-small --out artifacts/verification/api-smoke.json`

必须指向本机匹配发现记录的短命测试服务。工具先验证同连接HMAC proof，再发送Bearer；独立解析JSON/SSE，报告每项pass/fail/skipped，不调用服务端DTO/编码器作测试oracle。该命令末尾执行真实shutdown并等待原实例释放，不能用于希望继续保留的生产服务。可选`--disconnect-cycles N`默认5，允许1..50，用于有限断连后恢复压力验证；任何首次失败立即停止循环且保留已记录结果。

跨平台真实短命服务编排：

```
python scripts/run_api_smoke.py --cli target/debug/ai-runtime.exe --xtask target/debug/xtask.exe --model models/qa-small.gguf --out-dir artifacts/verification/windows-api-cli
```

Linux去掉.exe并使用实际Cargo target路径。先构建runtime-cli、xtask及runtime-worker；wrapper只创建临时私有data/credentials，端口0，由实际CLI在线导入固定模型，smoke显式cpu/context2048/threads2/batch128，最后确认服务退出。模型hash事先由baseline-verify/固定fixture核验；报告继续记录sha256。shared开发环境将TMPDIR设到空间充足的专用临时根，避免/tmp tmpfs容量污染测试。


## T05 Windows Release 产品与独立验收包

原生 Windows x64 开发机上执行：

```powershell
cargo run --locked -p xtask -- build --platform windows-x64 --backend cpu
```

`xtask/src/windows_package.rs`委托`scripts/package_windows.py`，要求固定Rust1.98.1、CMake4.4.3及既有VS2022 C++/Release Redist。脚本不下载/安装工具、模型或运行库；不接受Linux交叉构建冒充Windows包。复用`build/native-release`，验证同一VS实例、配置、x64/CPU/CRT和精确archive身份后增量检查原生目标。Rust Release使用专门target目录；先缺native独立构建管理CLI，再构建worker和独立验收器。

输出：

- `dist/windows-x64-cpu/`、`dist/windows-x64-cpu.zip`、`dist/windows-x64-cpu.zip.sha256`
- `dist/acceptance-tools/`、`dist/acceptance-tools.zip`、`dist/acceptance-tools.zip.sha256`
- `artifacts/verification/windows-package/build-result.json`、`pe-inspection.json`；这是构建/闭包证据，不是运行验收
- 存在时的PDB位于`build/windows-x64-cpu/cargo/x86_64-pc-windows-msvc/release/`，不混入两份ZIP

产品与工具各自提供真实PE闭包所需的app-local CRT、许可、manifest/SHA256SUMS。逐文件白名单/大小/hash/路径/Release/架构检查完成后才发布输出目录。源工作树dirty可用于本地诊断记录；CI供本地验收的产物必须为精确GITHUB_SHA且project_dirty=false。模型与临时token/data不进入包。

目标用户只运行预编译工具，完整接口与覆盖见[PACKAGE_ACCEPTANCE.md](PACKAGE_ACCEPTANCE.md)：

```powershell
.\acceptance-tools\nexa-acceptance.exe --model 'D:\模型 空格\Qwen3-0.6B-Q8_0.gguf' --out '.\package-report.json' --machine-role target
```

默认产品在工具目录旁`../windows-x64-cpu`，可显式`--package DIR`；模型与报告必须提供。CI显式传解压产品目录，验收器核对后将包内真实CLI传给共用api-smoke，不依赖xtask位置猜测。发行必测CLI缺失直接失败。`api-smoke`开发入口新增`--cli ABS_PATH --release-acceptance true`供这一严格路径使用；一般T04入口保留其已有范围。独立验收默认五次断流恢复，`--disconnect-cycles 1..50`可显式指定；T05 Release CI明确传50，原有T04 debug CI也保留50次。

所有产品进程在自有空临时CWD和仅系统目录PATH启动，不能借验收器自身DLL补产品依赖；工具也仅持有短命测试凭据。退出0只表示短程包检查过，A20/Win10实际build/无开发工具/VC预装/实际离线证据分别记录，skipped/unverified不视为通过；当前T05/T06门槛以用户批准的范围调整和状态记录为准。

独立开发检查（不要求目标机执行）：

```sh
python -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'
rustc --edition 2024 --test crates/llama-adapter/native_identity.rs -o build/native-identity-tests
build/native-identity-tests
python scripts/stage_ci_evidence.py
```

Windows独立identity测试程序后缀为`.exe`。证据stage只接受闭合已审查报告，保留状态、参数、身份与脱敏前后hash；上游合成正文只留hash/字节数/fixture关联。拒绝内容产生安全失败report并exit1，不上传原始整个目录。真正Windows/目标机结果见[T05记录](../docs/verification/2026-10-01-t05-windows-package.md)。

2026-10-01，源码6a7e9d0的[Windows Release CI36829233039](https://github.com/Naza3/Nexa/actions/runs/36829233039)已完成上述打包与解压后的真实独立验收：包检查16pass/2范围skip，HTTP89pass/9skip，50次断流全部实际执行并恢复。四类路径含中文/空格，受限PATH/空CWD、服务回收与自有data清理通过；用户Windows10 build19044 / i5-8400短验随后通过：16项包检查、HTTP44pass/9skip、5/5/5断流；产品EXE/源模型含中文但无空格，临时CWD/data含中文与空格。用户声明已有开发工具且测试联网，用户批准将A20无开发工具/离线与长期稳定性留作后期验证，T05按当前范围收口并继续T06。
