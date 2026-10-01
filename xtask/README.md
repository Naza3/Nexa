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

执行规格中后续阶段的`check`、`test --suite contract`、`build`尚未实现，调用会明确失败。T04已新增api-smoke，范围与关停副作用见下节；每阶段实际验收分别记录，不能凭一条命令宣布T02–T09全完成。


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
