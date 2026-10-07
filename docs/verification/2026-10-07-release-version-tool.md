# W05-VERSION-1：一键更新版本与发布构建修复

日期：2026-10-07。状态：源码及 Linux 开发验证已完成，新的 Windows 原生运行待验证。用户要求解决 Actions 失败并增加统一版本更新工具。

## 基线与根因

开发分支 `codex/dev` 快进同步 main `2abb7bef7329032fe755f19dc3605c863949cb4c`。该提交已把 Cargo/Tauri/npm 产品版本升至 `0.2.0`，但 `apps/desktop/package-lock.json` 顶层与 `packages[""].version` 仍为 `0.1.0`。抽取该提交的版本来源并调用原 `repository_version()`，实际复现 `Cargo/Tauri/npm/package-lock versions must all match the committed workspace version`。

同步 npm 锁后，原发布测试又出现一个失败：`test_each_declared_version_and_local_lock_is_checked` 替换 Cargo.lock 第一个 `"0.2.0"`，实际命中桌面锁中的 registry 包 `core-graphics-types`，没有改变本地产品包。生产校验器正确忽略第三方版本，故测试期待抛错却未抛错。修复测试为按无 `source` 的本地条目变异，并加入第三方与产品同版本碰撞回归；没有放宽生产发布校验。

两次 v0.2.0 tag run 在检查期间已无法读取，GitHub 返回 404，因此不宣称获得了它们的完整失败日志。另一个可读取的 [Actions37599775078](https://github.com/Naza3/Nexa/actions/runs/37599775078)（精确 `857bdfe96c94cf29f913b4dd62b04880be0806df`）在 `Build pinned upstream and shim` 失败：CTest 前四项通过，第五项 `air-ocr-template-test` 因找不到 EXE 未运行。`build_windows_ci.py` 固定目标漏掉该新增测试。补齐该目标，并从真实 CMake 测试注册校验实际构建命令包含全部测试 executable；未删除测试或改成跳过。

## 实现范围

- `update-version.cmd`：Windows 双击交互输入、空输入取消、命令行转发、Python 3.11+ 检测，保留退出码；仓库属性固定 CRLF
- `scripts/set_version.py`：离线标准库工具，从脚本路径定位仓库，接受 `MAJOR.MINOR.PATCH` 或 `vMAJOR.MINOR.PATCH`，复用既有 MSI 版本限制；支持预览、一致性检查及修复部分手工升级
- 七个版本文件先生成候选，在临时目录使用生产门禁验证，再逐文件替换；普通写入失败尝试回滚原字节。只更新 Cargo 无来源本地包及其版本限定边、npm 根身份、产品 manifests，不重解依赖、不改第三方 checksum/source、不执行 Git/tag/发布
- 本轮实际产品版本保持用户选择的 `0.2.0`；npm 工具同步锁文件只产生两处版本差异。示例 `0.2.1` 仅在临时测试目录或预览中运行

这是开发辅助工具和构建修复，应用行为及发行规则沿用既有规范。七个文件的更新不承诺进程被强制结束或断电时的整体事务；异常中断后可以用同一版本重新同步并检查 diff。

## 实际验证

以下命令在 `/workspace/Nexa` 的 Linux 云开发环境执行。Rust/CMake 使用既有 `/workspace/onboarding/nexa-env.sh` 激活的固定工具链。

| 命令或验证 | 结果与退出码 |
| --- | --- |
| `npm install --package-lock-only --ignore-scripts --offline --prefix apps/desktop` | 退出 0；仅 npm 根版本两处从 0.1.0 到 0.2.0 |
| `python3 -B scripts/set_version.py --check` | 退出 0，版本 0.2.0 |
| `python3 -B scripts/set_version.py 0.2.0` | 退出 0，0 文件改变，幂等 |
| `python3 -B scripts/set_version.py v0.2.1 --dry-run` | 退出 0，报告七文件，源码版本未改变 |
| `python3 -B -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'` | 288 项，283 通过/5 平台 skip，退出 0；日志 `/tmp/nexa-version-python-tests.log` |
| `cargo metadata --offline --locked --format-version 1 --no-deps` | 退出 0；13 个本地 workspace 包均 0.2.0 |
| 同上并加 `--manifest-path apps/desktop/src-tauri/Cargo.toml` | 退出 0；独立桌面 workspace 包 0.2.0 |
| 新 CMake 目标覆盖测试与旧目标列表 | 精确复现 OCR 漏项，退出 1；修复后同一 CI suite 10 项通过/退出 0 |
| 使用实际 `NATIVE_TARGETS` 执行 `cmake --build build/native-ocr --config Release --target ... --parallel 4` | Linux 增量编译成功，退出 0；日志 `/tmp/nexa-ci-native-build.log` |
| `ctest --test-dir build/native-ocr -C Release --output-on-failure` | 5/5 通过，退出 0；日志 `/tmp/nexa-ci-native-ctest.log` |

新工具用真实版本文件的临时副本验证：全部七文件同步、tag 输入、无效版本拒绝、同版本幂等、预览不写、部分错配修复、损坏输入不部分写入、第三方版本碰撞与依赖保持、逐文件 LF/CRLF、第二次写入错误回滚、CLI 异目录和交互行为。Windows 专属 cmd 入口测试检查含空格路径、正常同步、检查模式及非法版本退出码，Linux 下明确 skip，由既有完整 Windows Python suite 执行。

独立只读审查还通过同名同版本 local/registry 包及来源限定依赖边反例、全 CRLF、第三次写入失败恢复；这些为分层证据，不累加到 288 项计数。未发现剩余源码阻断。

## 未验证条件与下一步

此处不把 Linux CTest、Cargo metadata 或 `.cmd` 静态审查当成 Windows 成功。新的完整原生 Windows 构建、三格式包、安装器生命周期、用户 Windows 10/i5-8400 双击使用仍待验证。临时日志不作为发行产物提交。

推送开发分支后由既有 Actions 验证精确提交。发布时须先提交并推送七文件版本变更，再由维护者在包含全部修复的提交上创建匹配版本 tag；重跑指向旧源码的 tag 不会获得新代码。本轮没有移动已有 tag 或发布 Release。
