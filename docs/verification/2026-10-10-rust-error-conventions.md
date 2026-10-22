# Rust 错误规范落地验证

- 任务：W05-RUST-ERROR-1
- 基线：codex/dev `9b18f41d162770873364b27eea7406b73c4abdd5`
- 状态：规范及两个类型化切片完成，本地主机验证通过；未做Windows发行验证

## 范围

1. 新增 `docs/rust-error-handling.md`，AGENTS 与索引关联。
2. 根 workspace 各成员继承 unused_must_use / dbg_macro deny，独立桌面壳相同；增加两项 Python 清单检查以防成员遗漏。
3. 启动错误保留类型和 source；固定启动码按类型映射，不调用任意 Display，source 遍历上限16。
4. 私有文件保护和发布后持久化未确认改用 io::Error 内的具名类型；五处上层文本分类改为 downcast，公开 DTO 和原错误码保持。
5. 添加伪装同文案、源链/脱敏、循环 source、实际缺失 worker、Unix 真实发布前后失败等回归。

## 当前验证

- `git diff --check`：退出0。
- `python3 -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p test_rust_error_policy.py`：2项通过，退出0。
- 首轮全部 Python：372项，3个 error、5个 skip，退出1。错误是新云端 checkout 尚未拉取锁定 llama.cpp 子模块中的 stb/miniaudio 许可文件；不是已完成的全量验证。已恢复锁定子模块 2149c00f4442dc59302e134a02e4c99d5f7ed9fc，重跑同命令退出0：372项中367通过、5平台skip。未调整门槛。
- Rust：环境重建后无工具链，首次安装审批取消后停止；用户随后明确批准，在隔离目录安装官方1.98.1及rustfmt/Clippy，未改用户电脑。以下全部命令退出0。

- 独立静态审查：未发现类型/调用点、脱敏、Unix发布语义或Windows共用类型的阻断问题；不代替编译。

## Rust 实际命令和结果

- `cargo fmt --all -- --check` 与 `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check`：通过。
- `cargo test --locked --workspace --exclude llama-adapter --exclude engine-host --exclude runtime-worker --all-targets`：631 passed、0 failed、4既有ignored；覆盖本轮修改的管理端/配置/桥及其依赖。
- `cargo clippy --locked --workspace --exclude llama-adapter --exclude engine-host --exclude runtime-worker --all-targets -- -D warnings`：通过。
- `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets`：Linux壳33 passed、0 failed。
- `cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings`：通过。
- 两 Cargo.lock 没有变更；工作树无其他人的已有修改。fetch确认main `1c3650c352e56472f6ec7a5880b519f905ea6f8a` 是当前开发基线祖先，无需合并冲突。

验证宿主 x86_64-unknown-linux-gnu；Rust1.98.1。开发环境证据目录 `/workspace/shared/nexa-rust-tooling/logs/2026-10-10-errors/` 含逐命令日志、results.tsv 和157项源码hash清单，便于当前会话核对；这些本机路径不是永久下载链接。

## 未验证条件与发布限制

本轮不包含原生推理三个crate的完整workspace构建，缺CMake/Ninja的环境限制不能省略成“全部Rust通过”。Linux桌面壳不会执行cfg(windows)的真实Tauri、ACL及安装器代码。Windows交叉编译/原生运行、安装/ACL、用户目标设备没有新增验证。

本轮只本地中文提交，无推送、无新包、无版本/Release变更。后续如获准推送，必须在同源码完整Windows交叉门槛之后，不能沿用上轮9b18f41作为本轮二进制证据。

## 范围边界

本次建立全项目规则并落地两个错误分类切片，没有宣称所有历史错误均完成重构。已发现后续审查点：bridge library/download 的部分 blocking join 仍映射 write_failed，涉及可能已发布的结果，应独立核对副作用与回归；现有共享状态锁的 unwrap 不能简单改成 poisoned.into_inner 而隐含恢复不确定状态。
