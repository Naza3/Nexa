# Windows Tag 自动发行

## 触发与范围

`.github/workflows/native-windows.yml` 保留 `codex/dev` push 和手动构建，并增加 `v*` tag push。三条路径执行同一原生构建、真实固定 GGUF、完整桌面包及安装生命周期门禁；只有实际 tag push 能发布 GitHub Release。分支和手动运行只产生 Actions artifacts，不创建 tag、不发布版本。

工作流必须已经存在于 tag 指向的提交中。旧 `main` 或旧提交不会因为另一分支增加了此文件而自动获得新发行流程。先完成代码审查与该精确提交的构建；是否合并 `main`、选择哪个版本和实际推送 tag 由维护者决定，本次实现不会代替维护者创建版本 tag。

仅使用公开仓库的标准 `ubuntu-24.04` 和 `windows-2022` runner，不需要新增 secret、签名证书、收费 runner 或安装器框架账户。默认权限为 `contents: read`，所有 checkout 关闭持久凭据；只有依赖全部构建与验证成功的 `release` job 获得 `contents: write`。没有 PR 写权限工作流或 `pull_request_target` 入口。

## 版本规则

首版仅接受规范稳定版本 `vMAJOR.MINOR.PATCH`，例如 `v0.1.0`。前导零、第四段、预发布后缀（如 `-rc.1`）和构建后缀（如 `+build.1`）均拒绝，不会默默转换为相同 MSI 版本。三个数值上限分别为 `255`、`255`、`65535`；`0.0.0` 保留，不用作发行版本。

提交前统一这些文件，并更新两个 Cargo 锁文件中所有本地包版本：

- 根 `Cargo.toml` 的 `workspace.package.version` 与 `Cargo.lock`
- `apps/desktop/src-tauri/Cargo.toml`、`Cargo.lock`、`tauri.conf.json`
- `apps/desktop/package.json`、`package-lock.json` 顶层及根 package 版本

本地 workspace crate 继承版本，不允许静默覆盖成其他版本。CI 的 `release_version.py` 校验所有上述来源；它不会临时改版本、锁文件或污染源码身份。tag 版本必须等于已提交版本。版本升级后，应重新运行正常锁文件和应用回归，不手工将旧二进制标为新版本。

示例流程（仅说明，不表示这些命令已经执行）：

```sh
# 在包含新工作流、各版本一致、完整验证通过的精确提交上操作
# 确认当前 HEAD 是准备发行的提交，再由维护者创建并推送 tag
git tag -a v0.1.0 -m "Nexa 0.1.0"
git push origin v0.1.0
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

MSI 和 Setup 使用同一个 MSI payload；安装、修复、升级、失败回滚、降级拒绝、运行中进程保护、卸载及用户数据保留均由一次性 Windows runner 验证。Setup 另验证安装/修复/卸载、退出码和向导控制流。测试用升级/失败 fixture 不进入发行资产。具体安装语义见 [安装器说明](windows-msi.md)。

对应的修改后 aria2 源码、构建材料及第三方许可继续完整位于便携/安装后的 `download/` 闭包中，且同字节源码额外作为 Release 附件供获取。桌面、runtime、下载组件的原生身份和完整许可门禁保持；不会用GitHub自动源码快照代替 aria2 对应源码。

`release-manifest.json` 绑定精确 commit、版本、payload manifest、文件数量、每个资产的大小/SHA256及安装器验证结果。`SHA256SUMS` 覆盖四个分发资产和 release manifest。发布 job 下载同一 run 的精确 SHA artifact，再核验封闭库存及对应源码一致性。

## 失败、重跑与防覆盖

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
