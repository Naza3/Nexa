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

## 第二轮Windows路径修复

任务 W05-VERSION-2。实际推送的 `75ff0c0` 对应 [Actions37603689299](https://github.com/Naza3/Nexa/actions/runs/37603689299)，版本身份与下载组件通过，Windows 在 `Install locked Rust and compatible CMake tools` 内的 Python suite 失败：288 项中唯一失败为 `test_updates_all_seven_files_and_is_idempotent`，285 通过/2 平台 skip。快照使用 `str(path.relative_to(root))` 得到 `apps\desktop\...`，预期七文件常量为 `apps/desktop/...`，同一批修改因此被字符串集合误判；该测试后续幂等断言尚未执行。

失败日志中的预期 `Completion: failed` 和继承 PowerShell 环境诊断不构成其他测试失败；独立只读审查确认了其测试调用来源。Windows cmd 入口、换行及异常回滚用例均未失败。完整原生编译及安装器阶段尚未开始，不能转授前一轮 Linux CTest 结果。

开发分支快进同步用户合并并升至 `0.2.1` 的 main `997117b` 后，仅把快照键改为 `path.relative_to(root).as_posix()`。实际二进制读取、七文件精确集合、幂等及回滚断言保持。

- `python3 -B -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'`：Linux 288 项，283 通过/5 平台 skip，退出 0；日志 `/tmp/nexa-version-path-tests.log`
- 相同七条路径用 `PureWindowsPath` / `PurePosixPath` 构造反例：旧 Windows 字符串集合不等，POSIX 集合相等；修复后两者均等，退出 0。这是路径逻辑验证，不冒充 Windows 文件系统运行
- `python3 -B scripts/set_version.py --check`：0.2.1 一致，退出 0

下一步推送修复并检查原生 Windows 原失败步骤。版本升级来自用户，修复不改版本、tag、发布权限或测试准入规则。

### 精确修复提交的 Windows 步骤结果

修复提交 `cdbc12a98dfe7b4f596d77f22f25ef8a06ae86dd` 已推送，对应 [Actions37605103921](https://github.com/Naza3/Nexa/actions/runs/37605103921)。第一次尝试在更早的 aria2 策略门禁失败，诊断报告32项中31项通过，唯一失败为 `untrusted_certificate`：`self-signed.badssl.com` 返回 `exit=2`、`bytes=0`、`AbstractCommand.cc errorCode=2 Timeout`，没有取得预期 Schannel 证书拒绝证据。报告保存在本机 `/tmp/nexa-37605103921-evidence/windows-aria2-policy.json`，远端为该 run 的 `windows-cpu-native-cdbc12a98dfe7b4f596d77f22f25ef8a06ae86dd` artifact；未据此推断具体 TCP/TLS 故障阶段。

实际执行 `gh run rerun 37605103921 --repo Naza3/Nexa --failed`，仅对同提交失败 Windows job 重跑一次，复用已成功的来源构建。`gh api .../actions/runs/37605103921` 确认 `run_attempt=2`、源码 SHA 不变。第二次尝试中，组件门禁与 `Install locked Rust and compatible CMake tools` 步骤均已成功；后者包含完整严格 Python suite，并在非零退出时明确抛错，故此次 Windows 版本测试失败已恢复。独立审查确认 aria2 来源、补丁、探针及构建脚本与旧成功提交 `09b9e049` 相同，本轮未修改它们或放宽分类器。

记录时完整 CI 仍处于后续安装器/应用构建阶段，不宣称整体或新安装包成功。随后仅补记本段的文档提交与本次精确代码验证分开；无需为记录更新重复启动完整构建。

## 产品版本更新至0.2.2

任务 W05-VERSION-3。用户明确要求更新版本至 `0.2.2`。在含 OCR 配对与 UI 修复的 `e5741ac42cc12039e23004307c24c765e116e9fb` 上使用既有工具同步七文件；最新 main `02c90da` 已包含在开发分支中。本批没有新增工具逻辑或依赖变更。

以下命令退出码均为 0：

- `python3 scripts/set_version.py 0.2.2`：同步七文件；根 Cargo 锁13个本地包、桌面锁10个本地包随产品版本更新。
- `python3 scripts/set_version.py --check`：确认所有产品版本为 `0.2.2`。
- `python3 scripts/set_version.py 0.2.2 --dry-run`：0 文件需变化。
- `python3 -B -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_release*.py'`：39项，38通过/1个既有Windows专属跳过。
- 根工程执行 `cargo metadata --locked --offline --no-deps --format-version 1`，以及同命令添加 `--manifest-path apps/desktop/src-tauri/Cargo.toml`：13个根 workspace 包与1个桌面壳包均为 `0.2.2`，锁定解析通过。
- `git diff --check` 通过。独立结构化审查确认 Cargo 第三方包、依赖边和 checksum 不变；npm 锁仅顶层及根 package 版本变化。

本批证据限于版本同步和发行校验，不转授前一提交的完整构建结果。按既有授权推送 `codex/dev`，新版本 Windows 构建由现有分支 CI 验证；未创建或移动 tag、未发布 Release。

## 产品版本更新至0.2.3并创建tag

任务 W05-VERSION-4。用户追加明确授权“升级版本0.2.3，创建tag”。基线为已推送`2770b09`，包含OCR选图预览、状态反馈与tag只读缓存；构建优化前置提交843已通过完整Windows流水线。main仍为`1c3650c`且是当前祖先，本批不自行合并main；新tag绑定包含全部修复的开发分支提交。

实际本地检查：`python3 scripts/set_version.py 0.2.3`同步七文件、`python3 scripts/set_version.py --check`确认0.2.3、同版本`--dry-run`为0文件，退出码均0。严格编码的`python3 -B -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_release*.py'`39项（38通过/1个Windows专属skip），退出0。远端`git ls-remote --tags origin refs/tags/v0.2.3`为空，本地无同名tag，GitHub Release查询返回not found；不移动旧tag，也不预建空Release。独立结构化审查通过：根Cargo锁13个本地包、壳锁10个本地包升为0.2.3；142/447个第三方包的结构、checksum与依赖边保持。固定环境下两次`cargo metadata --offline --locked --no-deps --format-version 1`（第二次加`--manifest-path apps/desktop/src-tauri/Cargo.toml`）退出0，13+1个workspace成员全部0.2.3。生产版本解析与MSI ProductVersion均为0.2.3，`git diff --check`退出0。

发布流程使用既有自动化：提交全部版本文件，按精确新提交验证`refs/tags/v0.2.3`与package版本，再创建附注tag并与开发分支推送。同一提交以tag流水线执行完整验证/发布，不并行保留冗余分支构建；正在运行的0.2.2中间构建由新版本取代，不据此宣称其完整成功。当前main未包含缓存工作流，因此新tag可能冷构建；不为缓存擅自合并main或读取兄弟分支缓存。

七文件与当时验证记录已提交为`6309dedf6e15019a93f51ca24a27442850e7575d`。清洁工作树下实际调用生产`release_windows.validate`，传入该HEAD及`refs/tags/v0.2.3`，退出0并返回版本0.2.3、tag v0.2.3与同一commit。已执行`git tag -a v0.2.3 -m 'Nexa 0.2.3：修复 OCR 图片预览并优化构建'`及`git push --atomic origin codex/dev refs/tags/v0.2.3`，均退出0。远端附注tag对象为`cc1d97c3567933d8a09257717f6fd4573cc8c6c0`，解引用精确为6309ded。

[正式tag Actions37638109341](https://github.com/Naza3/Nexa/actions/runs/37638109341)已启动且版本身份通过。同提交重复分支37638109547的`gh run cancel`请求退出0，随后状态cancelled。旧0.2.2中间运行37636935709已由分支并发规则取消，补发cancel命令因其已完成返回1；只读查询确认cancelled，其中desktop-build成功、runtime中止。最终验证以tag运行记录；没有取消或修改旧版本正式发布流程。当前没有提前创建v0.2.3 Release，待既有自动化通过所有门禁后创建完整资产。

### 0.2.3 原生结果与发布阻塞

[Actions37638109341](https://github.com/Naza3/Nexa/actions/runs/37638109341)精确源码6309ded的五个实际job全部成功：版本身份、同源下载组件、桌面构建、Runtime构建和最终native。Windows前端850项/桌面壳33项、原生真实模型、HTTP/CLI、两份handoff、桌面bridge、三格式包、13项安装生命周期及完整来源/许可闭包均通过。原生门禁完成于15:09:16 UTC，整体运行在15:11:00 UTC被平台标为failure，**不能写成整轮Actions成功**。

Release job尚未创建，GitHub网页唯一workflow级annotation为`Internal server error. Correlation ID: 6f252605-a6fb-4f37-820c-f04bc75fee88`；五个job的checks均无错误annotation。两次执行`gh run rerun 37638109341 --failed`都返回HTTP500、退出1，run_attempt仍1，没有重编、移动tag或替换产物。全局GitHub状态页当时未列出事故，不能据此否定该run明确记录的内部错误。证据：`/tmp/nexa-v023-workflow-platform-error.html`及同名`.txt`。

已从同run下载[发行资产11492458105](https://github.com/Naza3/Nexa/actions/runs/37638109341/artifacts/11492458105)，归档SHA256为`47e14f9a8a18f41dce43ac3d83aa09c0c0f42973d7ae342f6f536710f931f3ee`。`python3 scripts/release_windows.py verify --directory /tmp/nexa-v023-audit --commit 6309dedf6e15019a93f51ca24a27442850e7575d --version 0.2.3`退出0，六资产集合、SHA256SUMS、四分发文件大小/hash、portable闭包、对应源码和安装器证明全部通过。原始artifact ZIP还按GitHub API摘要逐字节核对，再与六个下载文件比较，全部一致。审计报告：`/tmp/nexa-v023-audit-report.json`。

为了恢复用户tag触发的发行，使用同一生产`release_windows.publish`尝试复用这些原始文件。临时恢复器`/tmp/nexa_recover_v023_release.py`先验证实际GitHub run为tag push、精确repo/commit、五个成功job、artifact来源与摘要、全部六文件和生产verify，并确认发布脚本与tag源码完全相同；环境事件字段来自实际run与release-identity原始tag日志，不构造不存在的构建成功。预检退出0。生产发布逻辑仍要求附注tag解析、封闭库存、draft内上传、每个远端digest及发布前后tag重验；不重编或放宽条件。

补发创建了带精确commit/manifest marker的草稿（release id405895004），但`uploads.github.com`第一次上传返回HTTP401，生产脚本退出1并保留草稿。随后用环境既有CLI执行`gh release upload v0.2.3 /tmp/nexa-v023-audit/SHA256SUMS --repo Naza3/Nexa`也返回401、退出1。已按runtime指引复查环境状态和网络策略，未更换身份、登录、修改凭据/代理或关闭TLS。草稿仍为`draft=true`、assets=[]，没有不完整公开Release；旧v0.2.2未修改。

最终状态：版本0.2.3和附注tag已完成并推送，Windows产物通过验证；**公开Release受GitHub工作流内部500和当前上传认证401阻塞**。下一步在平台恢复后优先恢复原run的发布阶段，或通过维护者有效上传凭据使用此六文件完成草稿；应复用原始字节，不重建后强行复用不同manifest。Windows10/i5-8400原生界面、干净机器/离线等独立条件仍待验。

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `Nexa-0.2.3-aria2-1.37.0-nexa-corresponding-source.tar.gz` | 5740619 | `10919949c3be443b17e1d04abcc3e54b2f416a8103277ba0f984a7a7566a7335` |
| `Nexa-0.2.3-windows-x64-portable.zip` | 16865319 | `9e0be4bc4323c409646ac78a07367ab9c48e86aa719179c5536d4e5238f14388` |
| `Nexa-0.2.3-windows-x64-setup.exe` | 14540800 | `d39bd99b26bdda78a2aa6e902e9730ca9940eef12df371e90afdd025a1c6977b` |
| `Nexa-0.2.3-windows-x64-setup.msi` | 14524416 | `7e2cd5f3eb24ba5210dc80e1114ae3ec5bbd39b0af9b4b14cbacefc9f8d494ee` |
| `SHA256SUMS` | 511 | `915550198da6e41c09da40ce3a83b92c94249616c116accf6718c9f886953688` |
| `release-manifest.json` | 2238 | `8c0076dcf00b5e5ae865b499699176e63e6da87b27955edadca30b5464db7ab6` |
