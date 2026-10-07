# Windows CI 构建优化验证

任务：W05-CI-PERF-1。状态：首轮构建/测试及交接通过，最终打包失败已修复，原生复验中。用户授权按分析方案依次优化；开发分支已快进同步 main `1c3650c352e56472f6ec7a5880b519f905ea6f8a`，产品版本保持 0.2.2。

## 基线

[Actions 37616762525](https://github.com/Naza3/Nexa/actions/runs/37616762525)（`bf8c03f`）成功运行，从创建到结束 36 分 56 秒，native job 33 分 10 秒。Tauri Rust 检查/Release 525 秒、原生构建 279 秒、workspace Rust 检查 194 秒、Release 包 143 秒、桌面打包及另编 harness 105 秒（其中 harness 92 秒）。此前仓库 Actions cache 查询为 0 项。另外两次成功 native job 为 31 分 14 秒和 32 分 04 秒。

这些是旧提交的测量，不作为新构建完成或加速比例的证据。

## 修改

实现范围与边界见 [ADR0033](../decisions/0033-parallel-windows-builds.md)：缓存依赖/独立 Rust target，Release harness 复用、前端构建去重、桌面/Runtime 并行、同 run 精确 handoff 验证及最后统一安装器门禁。官方 `actions/cache` v6.1.0 固定提交 `55cc8345863c7cc4c66a329aec7e433d2d1c52a9`，已读取其 `action.yml` 确认原生 `node24`；既有四个 Action 的 Node24 SHA 保持。

补充检查了 GitHub 官方缓存范围：只读当前 ref 和默认分支，不跨不同 tag/兄弟分支。因仓库默认分支为 main，追加 main push 构建用于合并后验证和 tag 缓存预热；编译缓存增加同角色/同工具链回退，以便版本/锁文件变化后仍复用有效依赖。冷构建期间不推送这个后续调整，避免取消正在测量的运行。

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

## 首轮原生执行与冷依赖修复

精确提交 `d7b84cef341bb3ed72fe726f493fec841f49d408` 的 [Actions37627550537](https://github.com/Naza3/Nexa/actions/runs/37627550537) 从 13:19:11 UTC 到 13:51:42 UTC，共32分31秒，**结果失败，不能记为完整构建耗时或成功加速证据**。桌面 job14分37秒、Runtime job25分23秒均成功，完整 Rust/真实模型/HTTP/CLI/独立解压门禁及两份 handoff 恢复已通过。

新 assembly runner 未命中桌面 Cargo 下载缓存，首次 `cargo metadata` 下载依赖时将进度写入 stderr。原 `command()` 合并 stdout/stderr，导致 JSON 解析在第一个字符失败。已让两处 Cargo metadata 只解析 stdout，另保留 stderr 诊断，非零退出仍拒绝。新增4项真实子进程回归覆盖下载噪音、非零状态、无效JSON和默认合流兼容；严格完整 Python 更新为319项（314通过/5平台跳过），退出0，actionlint、版本0.2.2和差异检查均退出0。main触发及缓存前缀调整的35项针对性测试也通过。

缓存未命中原因已核实：桌面 runner image为`20260927.320.1`，Runtime和assembly为`20261004.326.1`。两个不同的新image VM得到相同缓存键；三者兼容工具链键相同，故交接正常通过。分区没有因每台VM随机失效，也不以放宽镜像分区掩盖冷依赖问题。

Release harness实际仅有47条`Compiling`，旧独立目录为122条，确实减少75条编译记录。但本轮较慢runner使总步耗时101秒，旧基线92秒，不能宣称单凭这次观测已节省秒数；剩余新增依赖及feature变体编译保留。

## Tag 缓存只读收尾

按用户补充要求，tag不保存独立缓存。五处Cargo缓存拆分官方同SHA的restore/save，所有save都要求success、非tag、主键非空且没有精确命中；两处setup-node仅非tag传入npm缓存，tag使用npm实际缓存路径与官方同一key恢复，不注册自动保存。现有main/dev缓存键、前缀、路径和其余构建门禁经结构化逐项比较保持。GitHub优先当前ref再main的服务规则及历史同tag缓存边界见ADR0033，不声称restore具有不存在的ref输入。

实际检查：`/tmp/nexa-actionlint/actionlint .github/workflows/native-windows.yml`退出0；`python3 -m unittest discover -s scripts -p 'test_windows_ci.py'`14/14通过、退出0；`git diff --check`退出0。固定官方cache/restore、cache/save的action.yml与setup-node源码已核对；静态遍历确认没有cache主动作、所有显式save对tag不可达、tag setup-node未启用任何缓存保存路径。此检查不等同已执行新的tag发布，本次不创建tag或Release。
