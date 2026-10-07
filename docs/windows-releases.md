# Windows Tag 自动发行

## 触发与范围

`.github/workflows/native-windows.yml` 保留 `codex/dev` push 和手动构建，并增加 `v*` tag push。三条路径执行同一原生构建、真实固定 GGUF、完整桌面包及安装生命周期门禁；只有实际 tag push 能发布 GitHub Release。分支和手动运行只产生 Actions artifacts，不创建 tag、不发布版本。

工作流必须已经存在于 tag 指向的提交中。旧 `main` 或旧提交不会因为另一分支增加了此文件而自动获得新发行流程。先完成代码审查与该精确提交的构建；是否合并 `main`、选择哪个版本和实际推送 tag 由维护者决定，本次实现不会代替维护者创建版本 tag。

仅使用公开仓库的标准 `ubuntu-24.04` 和 `windows-2022` runner，不需要新增 secret、签名证书、收费 runner 或安装器框架账户。默认权限为 `contents: read`，所有 checkout 关闭持久凭据；只有依赖全部构建与验证成功的 `release` job 获得 `contents: write`。没有 PR 写权限工作流或 `pull_request_target` 入口。

## 版本规则

首版仅接受规范稳定版本 `vMAJOR.MINOR.PATCH`，例如 `v0.1.0`。前导零、第四段、预发布后缀（如 `-rc.1`）和构建后缀（如 `+build.1`）均拒绝，不会默默转换为相同 MSI 版本。三个数值上限分别为 `255`、`255`、`65535`；`0.0.0` 保留，不用作发行版本。

提交前使用仓库根目录的 `update-version.cmd` 一次同步版本。Windows 安装 Python 3.11 或更新版本后，双击该文件，输入 `0.2.1`（也接受 `v0.2.1`），按回车即可；输入为空则取消。PowerShell 中也可以运行：

```powershell
.\update-version.cmd 0.2.1
.\update-version.cmd 0.2.1 --dry-run  # 只预览将修改的文件
.\update-version.cmd --check         # 检查当前版本是否一致
```

跨平台直接运行同一个 Python 工具（Linux 使用 `python3`，Windows 也可使用 `py -3`）：

```sh
python3 scripts/set_version.py 0.2.1
```

工具离线运行，不需要安装 Cargo、npm 或第三方 Python 包；只同步以下七个文件中的产品版本：

- 根 `Cargo.toml` 的 `workspace.package.version` 与 `Cargo.lock`
- `apps/desktop/src-tauri/Cargo.toml`、`Cargo.lock`、`tauri.conf.json`
- `apps/desktop/package.json`、`package-lock.json` 顶层及根 package 版本

`Cargo.lock` 仍由 Cargo 管理依赖解析，工具只更新无 `source` 的本地包版本及其版本限定引用；npm 锁文件只更新顶层及根 package 的版本。第三方依赖、校验和与下载地址不变。工具可修复之前手工修改造成的版本不一致；写入前在临时目录调用同一发布校验器，检查失败不写入，普通写入异常会尝试恢复原文件。它保留每个文件的 LF/CRLF，重复同步同一版本不会产生额外修改；没有跨七文件的断电事务保证。

本地 workspace crate 继承版本，不允许静默覆盖成其他版本。CI 的 `release_version.py` 校验所有上述来源；它不会临时改版本、锁文件或污染源码身份。工具不会提交、推送或创建 tag；查看 `git diff`、提交全部版本变更、完成应用回归后，再在包含该修改的提交上创建同版本 tag。仅重跑旧 tag 的 Actions 不会包含新修复。

示例流程（仅说明，不表示这些命令已经执行）：

```sh
# 先用上面的工具同步版本，审查、提交并推送修改，完成构建验证
# 确认当前 HEAD 包含这些修改，并且该版本尚无正式发行，再创建 tag
git tag -a v0.2.1 -m "Nexa 0.2.1"
git push origin v0.2.1
```

## 同源三格式与发布资产

一份已经通过 `package_desktop_windows.py` 和真实 bridge 验收的 `dist/desktop-windows` 是唯一应用 payload。安装器直接包装这些字节，不第二次重编桌面、runtime 或下载组件。便携 ZIP 保留原字节，仅将外部资产文件名改为带版本的名称；内部目录仍为 `desktop-windows/`。

Windows 工具链准备后先编译安装器辅助程序与资源，提前发现编译/系统依赖问题。此门禁只产生 `compile-pass`，明确 `installation_tested=false`；完整应用 payload、MSI 和生命周期验收仍在后面的真实阶段执行。

同一 Release 包含：

- `Nexa-<version>-windows-x64-portable.zip`
- `Nexa-<version>-windows-x64-setup.msi`
- `Nexa-<version>-windows-x64-setup.exe`
- `Nexa-<version>-aria2-1.37.0-nexa-corresponding-source.tar.gz`
- `release-manifest.json`
- `SHA256SUMS`

MSI 和 Setup 使用同一个 MSI payload；安装、修复、升级、失败回滚、降级拒绝、运行中进程保护、卸载及用户数据保留均由一次性 Windows runner 验证。Setup 另验证安装/修复/卸载、退出码和向导控制流。测试用升级/失败 fixture 不进入发行资产。具体安装语义见 [安装器说明](windows-installers.md)。

对应的修改后 aria2 源码、构建材料及第三方许可继续完整位于便携/安装后的 `download/` 闭包中，且同字节源码额外作为 Release 附件供获取。桌面、runtime、下载组件的原生身份和完整许可门禁保持；不会用GitHub自动源码快照代替 aria2 对应源码。

`release-manifest.json` 绑定精确 commit、版本、payload manifest、文件数量、每个资产的大小/SHA256及安装器验证结果。`SHA256SUMS` 覆盖四个分发资产和 release manifest。发布 job 下载同一 run 的精确 SHA artifact，再核验封闭库存及对应源码一致性。

## 失败、重跑与防覆盖

早期 Windows 下载组件检查依赖 `example.com` 和三个 badssl 公网样例。仅明确的零字节网络超时/连接中断会自动重试，最多三次，等待2秒、5秒；每次使用新目录，证书与策略通过条件保持。Actions日志显示样例名、原因与次数，`windows-aria2-policy.json` 保留每次尝试；未知错误、真实证书错误、策略失败不重试，公网持续不可用仍阻止构建。具体规则见[组件检查说明](../third_party/aria2/README.md#构建和检查)。

安装生命周期在执行过程中写入独立的脱敏诊断 sidecar。工作流在成功或失败后都尝试通过封闭 schema 校验，再上传固定的 `nexa-windows-installer-evidence-<commit>` artifact；只含预先审核的阶段、动作/窗口类别、退出码和计数。报告缺失也明确标记缺失，不伪造完成。未知字段、任意文字、用户路径或不一致 commit 被拒绝，校验失败不上传；原始 msiexec 日志和构建输出目录不进入该 artifact。runner 整体退出或任务超时导致后续步骤不能执行时，不能保证保留诊断。

若正式 MSI/Setup 构建已经成功而生命周期未成功，另保留 `nexa-windows-installers-UNVERIFIED-<commit>` 开发诊断 artifact，仅含当次正式 MSI、Setup EXE 和不含用户路径的 build report。它不是通过验收的发行包，不能作为验收通过或正式交付的依据；不含测试 fixture、原始日志、用户数据或整个 dist 目录。Release job 永不消费该 artifact，仍只接收完整验证后的 release 资产。

构建和验证通过后，发布脚本先通过 GitHub API 解析轻量/附注 tag，确认最终 commit 仍等于事件提交。缺失 tag 或移动 tag 均失败，不创建或修复 tag。

新版本先创建 draft，上传所有资产，核对服务器返回的 SHA256 和完整资产列表，再次核对 tag，最后公开发布。失败时保留 draft 便于诊断，不把不完整附件公开为成功发行。再次运行只允许恢复完全相同 commit、release manifest 和已有资产的 draft；不会覆盖任何已有资产。

这里的恢复指保留同一次构建的 artifact，重跑失败的 `release` job。MSI PackageCode、CAB/编译时间等可能随完整重建变化，同一 commit 不保证生成相同安装器字节；重跑所有 job 后如果 manifest 不同，流程会按设计拒绝复用旧 draft，而不是替换已有附件。构建 artifact 当前保留 7 天，须在过期前完成该字节集合的重试；过期或需要新字节时由维护者另行决策。

已公开的版本只有在 manifest 身份、完整资产集合、大小和服务端 digest 全部相同时才能作为无修改重跑成功。不同字节、缺失/未知资产或非本流程创建的同名 Release 均停止，交由维护者决定处理；脚本不删 Release、不替换附件、不强推 tag。

这只是本工作流的拒绝覆盖规则，并不声称仓库已经启用了平台级不可变 Release 或 tag 保护。相关安全设置没有在本轮更改。

## 验证边界

标准 Windows Server 2022 原生 CI 结果不能代替用户 Windows 10/i5-8400/16GB 机器验收。Setup 向导自动控制流测试不等于目标机视觉与真实交互验收；模型/配置保留也不表示任意历史版本间迁移均已覆盖。

当前发行不带模型、不自动下载 WebView2、不配置防火墙/开机启动；需要已有 Evergreen WebView2。安装程序未代码签名，Windows 可能提示未知发布者；未新增签名或时间戳服务。无开发工具的干净机器、实际离线、长期稳定性、Windows 11 和目标机 GUI 仍按已有路线单独验收。

## 官方规则来源

- [GitHub workflow tags、permissions 与 job 依赖](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax)
- [GitHub Release 创建、draft 与资产 API](https://docs.github.com/en/rest/releases/releases)
- [GitHub 平台级不可变 Release](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)
- [Microsoft MSI ProductVersion 三段及上限](https://learn.microsoft.com/en-us/windows/win32/msi/productversion)
