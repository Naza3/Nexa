# 当前可执行验证命令

在项目根目录执行；所有命令须使用 `rust-toolchain.toml` 固定工具链。模型须先按 `tests/fixtures/baseline.json` 所列固定 revision 获取，不提交模型。

```sh
cargo test --locked -p xtask
cargo run --locked -p xtask -- baseline-verify --model /path/to/Qwen3-0.6B-Q8_0.gguf --out artifacts/verification/baseline.json
cargo build --locked -p llama-adapter --example native-smoke
cargo run --locked -p xtask -- native-smoke --model /path/to/Qwen3-0.6B-Q8_0.gguf --bin target/debug/examples/native-smoke --out artifacts/verification/native-smoke.json
```

Windows 的 example 文件名为 `native-smoke.exe`。原生库构建配置以 `docs/build-lock.md` 为准，不通过 xtask 隐式下载或重建依赖。可用 `--device` 补充设备标签、`--manifest` 选择锁定清单；native-smoke 的 `--timeout-seconds` 为每个独立进程的时限，默认 180 秒。

`baseline-verify` 校验精确 Rust/llama 版本、llama 已跟踪源码无修改、真实 GGUF 文件 hash、架构、量化类型、原始模板 hash 和上下文边界。它仅验证基线身份，不能证明推理成功。

`native-smoke` 先执行相同基线校验，再调用已经构建的真实模型 example，检查中英文/长输入非空输出、预算、重复加载释放及取消/消费者停止路径。每个进程的退出码、时间、二进制/输入/输出摘要和观测数值写入 JSON。对子进程 stdout 做大小限制和独立结构断言；不会凭退出 0 或一条 pass 字段宣布成功。报告不保存原文或完整本地路径。重复加载只验证两轮功能生命周期；不宣称已测内存泄漏或性能。中途 prefill 取消、连续 100 请求、20 次加载卸载以及内存释放量均不在此 smoke 覆盖范围。

退出码：0 表示当前请求的验证范围通过；1 表示已执行的验证失败；2 表示参数/前置文件错误。读取不到文件时不会写一份伪成功报告。默认设备标签和无法测量的性能值为 `unavailable`。目标 Windows/Android 验收单独标记 `skipped`；即便在对应 OS 执行，也不能代替指定设备的完整验收。

执行规格中后续阶段的 `check`、`test --suite contract`、`build`、`api-smoke` 尚未实现，调用会明确失败。现在不能用它们宣布 T02–T09 完成。
