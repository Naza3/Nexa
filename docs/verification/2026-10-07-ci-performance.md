# Windows CI 构建优化验证

任务：W05-CI-PERF-1。状态：本地检查通过，原生 Windows 待验证。用户授权按分析方案依次优化；开发分支已快进同步 main `1c3650c352e56472f6ec7a5880b519f905ea6f8a`，产品版本保持 0.2.2。

## 基线

[Actions 37616762525](https://github.com/Naza3/Nexa/actions/runs/37616762525)（`bf8c03f`）成功运行，从创建到结束 36 分 56 秒，native job 33 分 10 秒。Tauri Rust 检查/Release 525 秒、原生构建 279 秒、workspace Rust 检查 194 秒、Release 包 143 秒、桌面打包及另编 harness 105 秒（其中 harness 92 秒）。此前仓库 Actions cache 查询为 0 项。另外两次成功 native job 为 31 分 14 秒和 32 分 04 秒。

这些是旧提交的测量，不作为新构建完成或加速比例的证据。

## 修改

实现范围与边界见 [ADR0033](../decisions/0033-parallel-windows-builds.md)：缓存依赖/独立 Rust target，Release harness 复用、前端构建去重、桌面/Runtime 并行、同 run 精确 handoff 验证及最后统一安装器门禁。官方 `actions/cache` v6.1.0 固定提交 `55cc8345863c7cc4c66a329aec7e433d2d1c52a9`，已读取其 `action.yml` 确认原生 `node24`；既有四个 Action 的 Node24 SHA 保持。

## 本地检查与原生结果

在仓库根目录执行：

| 实际检查 | 结果 |
| --- | --- |
| `python3 -B -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'` | 退出 0；315 项，310 通过 / 5 平台跳过 |
| `/tmp/nexa-actionlint/actionlint .github/workflows/native-windows.yml`（1.7.12） | 退出 0 |
| `python3 scripts/set_version.py --check` | 退出 0；0.2.2 |
| `git diff --check` | 退出 0 |
| PyYAML 解析新旧 workflow，比较具名门禁、三个未改 job、权限和并发取消规则 | 退出 0；旧门禁全部保留，release/版本/aria2 job 不变 |

315 项中包含 14 项 handoff 测试（使用真实包/许可验证、身份错配、字节篡改、ZIP 路径逃逸/重复/链接、源码漂移、大小上限、部分提升回滚）和 14 项 CI 规划测试。两个独立只读审查覆盖 job 依赖、assembly 依赖闭包、缓存及 Windows 环境大小写，未发现剩余阻断。当前没有把静态检查或启动 Actions 记为 Windows 成功；精确提交的冷/热缓存结果待运行补记。

限制：云环境为 Linux，不能本地执行 MSVC、Tauri Windows 和安装器；全部原生门禁由既有公开 Actions 承担。没有创建/移动 tag、发布新 Release 或合并 main。
