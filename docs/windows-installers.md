# Windows Tauri MSI 与 NSIS 安装契约

2026-10-08 用户确认改用 Tauri 官方 Windows 打包路线。本文件描述本次源码中的新安装器；本轮原生 Windows 安装、迁移和 GUI 验收尚未执行，不表示新安装包已经发布或通过实机验证。决定及与旧方案的关系见 [ADR0039](decisions/0039-tauri-windows-installers.md)。

## 用户选择与旧版迁移

同一个 `vMAJOR.MINOR.PATCH` Release 继续提供三种格式：

| 文件 | 行为 |
| --- | --- |
| `Nexa-<版本>-windows-x64-portable.zip` | 完整解压后直接运行，不登记安装产品 |
| `Nexa-<版本>-windows-x64-setup.msi` | Tauri 调用 WiX 生成，使用标准 WiX 安装/维护界面；支持安装、修复、事务升级和卸载 |
| `Nexa-<版本>-windows-x64-setup.exe` | Tauri 调用 NSIS 生成，使用标准 NSIS 向导；支持安装、同版本重装、升级和独立卸载器 |

**旧版 `Setup.exe` 内嵌 MSI；新版 `setup.exe` 是独立 NSIS 安装器。** 用旧 Setup 或旧 MSI 安装过 Nexa，应下载本次实现之后构建的新 **MSI** 原位更新。新 MSI 沿用旧产品族，支持从旧 `0.2.3` 安装替换至同为 `0.2.3` 的新构建，不要求先手工卸载。

已有新 MSI 的安装继续使用 MSI；已有 NSIS 的安装继续使用 EXE。两种新安装器会拒绝直接跨 MSI/NSIS 覆盖，避免同一目录出现两套卸载登记。确需换格式时，先通过 Windows 应用设置卸载 Nexa 程序，再安装另一格式；模型和用户数据保留。

新旧安装器可能使用相同版本号及文件名，应按来源提交、Release manifest 和 SHA-256 区分。既有 tag 附件不会因源码更新而改变，也不会被悄悄替换。便携版不登记 MSI/NSIS 产品，安装器不会自动迁移或删除便携目录。

所有格式目前都没有 Nexa 代码签名。SHA-256 验证字节一致性，不证明发布者身份；仅从可信的仓库 Release/对应 Actions 获取并核对摘要。Windows/SmartScreen 可能显示未知发布者提示；不要把绕过安全警告当成安装步骤。

## 安装位置、权限与数据

两种安装格式均固定安装到当前用户的 `%LOCALAPPDATA%\Programs\Nexa`，不请求管理员权限，不提供机器级安装，不修改 PATH、防火墙或系统服务。开始菜单只创建当前用户的 `Nexa\Nexa.lnk`，不提供自选安装位置，不创建桌面快捷方式。

- 配置、密钥、登记元数据等仍由应用放在 `%LOCALAPPDATA%\Nexa`。
- 默认模型位置是桌面 EXE 邻近的 `models`，在当前用户安装位置可写。
- 更新、修复和卸载只管理清单内程序文件、安装器登记与快捷方式；NSIS 另管理自己的 `uninstall.exe`。
- 安装目录内用户添加的 `models`、`model` 和其他非清单文件、应用数据目录及外部模型全部保留。
- 不做递归目录删除，只移除已空的安装器目录；卸载后含模型的 Nexa 目录会保留。
- 不自动迁移便携版模型或数据。历史数据格式的兼容范围仍由应用版本决定。

安装器自带的 EXE、DLL、manifest、许可证等属于受管程序文件。MSI 修复或 NSIS 重装会恢复这些文件；不要把它们用作用户数据。

安装、覆盖、修复或卸载前，先在 Nexa 中停止服务，再完全退出桌面程序。启用“关闭到托盘”后，仅关闭窗口不会退出；应使用托盘菜单的“退出 Nexa”。只读检查识别桌面、runtime、worker 和下载进程，包括可解析的短路径别名；发现运行中或无法确认状态时拒绝继续。安装器不强杀进程、不静默停止服务；MSI 关闭 Restart Manager 自动重启应用的行为。

目录检查拒绝安装根、受管文件和开始菜单路径中的重解析点或越界目标。MSI 路径比较可接受同一目标的 8.3 别名，拒绝 `.`、`..` 等改写；NSIS 还独立核对实际 `$INSTDIR`，`/D` 和卸载 `_?=` 不能扩大安装范围。检查与后续写入不是针对同一用户恶意并发改目录的安全隔离边界。

## 开机启动与卸载

安装器不会自动启用开机启动，也不会在安装完成后自动打开 Nexa。应用设置中的“开机启动”默认关闭；用户启用后，在登录 Windows 时打开当前 Nexa 主窗口。此选项与关闭到托盘独立。

启动设置直接读写当前用户 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 的 `Nexa` 字符串值，内容为带双引号的当前桌面 EXE 路径，不带参数，不维护另一份 TOML 副本。Windows 的“启动应用”设置仍可单独禁用该项。

同版本覆盖和高版本更新保留现有启动选择。真正卸载时，仅删除匹配本固定安装目录 `nexa-desktop.exe`、带双引号且无参数的 `Nexa` Run 值；若指向其他便携副本或无法确认归属，则不删除。不会删除整个 Run 注册表项。MSI 在成功卸载的 commit 阶段清理，并排除 `UPGRADINGPRODUCTCODE`；升级失败回滚不会提前删启动项。NSIS 替换使用 `/UPDATE` 保留启动项，普通卸载才执行归属检查。

## 前置组件与打包工具

目标仍是 Windows 10 x64 / i5-8400 / 16GB。Windows Server 2022 runner 的结果不能替代该设备、干净机器、离线和长期使用验收；Windows 11 仍按实际结果扩展。

Microsoft Edge WebView2 Evergreen Runtime 必须已安装。安装器不捆绑、下载或安装 WebView2；缺失时应用继续提供诊断，用户可从 [Microsoft 官方页面](https://developer.microsoft.com/microsoft-edge/webview2/) 获取。CLI runtime 不依赖 WebView2。

正式安装器使用锁定的 Tauri CLI `2.12.1`：MSI 路线调用 WiX `3.14.1`，EXE 路线调用 NSIS `3.11`。这是官方打包器及标准界面，配合保留 Nexa 固定目录、数据和进程边界的模板适配，不是原来的自研 Setup 向导，也不是完全不加配置的 Tauri 默认安装行为。

WiX 3.14.1 使用 MS-RL，不是 WiX 7 的工具/EULA 路线。Tauri、WiX、NSIS 及插件许可证原件和来源证据随既有桌面许可库存整合；详细工具版本、来源与上游插件构建身份限制见[模板说明](../packaging/tauri/windows/README.md)及[依赖证据](../packaging/tauri/windows/licenses/DEPENDENCY_EVIDENCE.md)。构建工具不随 Nexa 安装到用户机器。

应用载荷复用已验收的便携目录。打包使用隔离的元数据工程与输出目录，执行 `tauri bundle --no-binary-patching --no-sign`；不重编应用，不为区分安装格式修改主 EXE。runtime/worker 等按原相对路径装入，MSI 的资源片段使用 HKCU keypath 和空目录移除以满足每用户组件规则。安装器自身的界面、卸载器、元数据不属于便携应用字节；应用文件仍须逐字节一致。

## 同版本覆盖与失败恢复

MSI 固定 UpgradeCode 为 `{85615F8B-FD70-53D9-86ED-4164379CBC40}`。每次构建由 WiX 生成新的 ProductCode 和 PackageCode，沿用官方 `MajorUpgrade` 的同版本升级能力，并关闭降级。旧产品移除在 `InstallInitialize` 后进行，旧卸载与新安装处于同一 Windows Installer 事务；失败由 Windows Installer 回滚。重复运行同一原始 MSI 可进入标准维护界面，或使用 `/fa` 修复；修复应保留该次安装的 MSI 源文件。

NSIS 同版本重装或升级先检查现有登记与固定目录中的卸载器，执行其同格式卸载清理旧清单文件，再安装新文件，因此新清单删掉或改名的旧程序文件不会因简单覆盖而遗留。无法确定旧版本、缺少卸载器、登记与路径不符或请求降级时停止。

**NSIS 替换不提供 MSI 式事务回滚。** 旧程序卸载后若新安装失败，不能自动恢复旧程序；用户数据仍保留，修复失败原因后可重新运行安装包。这一限制也适用于同版本重装，不能用 MSI 回滚通过来代表 NSIS 可回滚。

同版本构建之间没有时间先后排序，新安装器允许用选定构建替换已有同版本；更低的三段版本仍被拒绝。旧安装器代码不会被追溯修改，后续同版本重装应使用本次修复后的安装器。允许本机覆盖不等于允许覆盖同名 tag/Release 附件。

## 命令行

普通用户双击新 MSI 或 EXE 使用相应标准向导，卸载可从 Windows 应用设置进入。以下 `0.2.3` 仅示例命令格式，须使用本次实现之后构建的实际新包，不表示已发布新附件：

```powershell
msiexec.exe /i Nexa-0.2.3-windows-x64-setup.msi /qn /norestart
msiexec.exe /fa Nexa-0.2.3-windows-x64-setup.msi /qn /norestart
msiexec.exe /x Nexa-0.2.3-windows-x64-setup.msi /qn /norestart
.\Nexa-0.2.3-windows-x64-setup.exe /S
Start-Process -FilePath "$env:LOCALAPPDATA\Programs\Nexa\uninstall.exe" -ArgumentList "/S" -Wait
```

新 EXE 的 `/S` 安装或重装均进入 NSIS 流程，不再使用旧自研 Setup 的 `/repair`、`/uninstall` 接口。MSI 使用 Windows Installer 的退出码；`0` 为成功、`1602` 为取消、`3010` 为成功但请求稍后重启。NSIS 的跨格式/降级拒绝返回 `1638`，路径或进程保护拒绝返回 `1603`；不要把旧 Setup 的全部参数和退出码约定套用到新 EXE。

## 构建与验收

生产命令：

```powershell
python scripts/package_tauri_windows.py --payload dist/desktop-windows --version <版本> --output dist/Nexa-<版本>-windows-x64-setup.msi --setup-output dist/Nexa-<版本>-windows-x64-setup.exe --report dist/windows-msi-build-report.json
python scripts/test_tauri_windows_lifecycle.py --payload dist/desktop-windows --msi dist/Nexa-<版本>-windows-x64-setup.msi --setup dist/Nexa-<版本>-windows-x64-setup.exe --report dist/windows-msi-lifecycle-report.json
```

保留旧报告文件名以接入既有发行流程，其内容已改为 Tauri 两种格式的来源与验收结果。旧 `package_windows_msi.py` 的自建 MSI 能力只用于私有旧身份迁移 fixture，不再生成正式发布安装器。

生命周期入口只允许一次性 GitHub Windows runner，拒绝已有 Nexa 程序/数据目录、开始菜单、安装登记或自启动项。检查包括旧 MSI 身份迁移、新 MSI 同版本变更载荷覆盖及写入后故障回滚、修复、高版本升级和卸载；NSIS 全新安装、原包重复安装、同版本增删文件替换、高版本升级和卸载；两格式降级拒绝、运行中拒绝、双向跨格式拒绝、用户数据/外部模型哨兵保留，以及开机启动的默认关闭、更新保留、卸载按归属清理。私有变更、未来版本和故障 fixture 不进入 Release。

`--check-toolchain --report <路径>` 只编译辅助程序并产生 `compile-pass`、`installation_tested=false`，不是安装测试；Linux 的 `--nsis-only` 仅检查交叉打包，不生成正式双格式发行结论。单元测试、WiX XSD 校验、Tauri 渲染/编译、原生生命周期和用户目标机 GUI 必须分别报告。本轮原生 Windows 尚未运行；旧自研 Setup 的历史成功不能转授给新 MSI/NSIS。

## 失败诊断与 MSI 宿主边界

生命周期为每个外部动作写入固定阶段、预期/实际数值退出码和 `windows-msi-diagnostics.json`。单次安装期限 240 秒；超时不强杀安装器，也不为了退出测试删除安装树。诊断只记录白名单 MSI 动作、错误码、窗口类别/控件 ID 和固定原因枚举，不包含原窗口文本、命令行、配置/令牌、用户完整路径或原始日志。工作流严格校验 schema/来源后才上传独立诊断目录；缺失报告不记通过。

MSI 保留自带 Windows 10 supportedOS manifest 的只读 OS EXE，并在事务前同步检查其退出码；DLL 继续负责目录/进程检查。`VersionNT64` 只作架构限制，不能用 MSI 宿主的兼容版本值替代 Windows 10 门禁。旧 DLL 曾在 Windows Server MSI 宿主中观察到 `6.3.20348`，迁移本次打包器不能恢复该错误判断；历史依据见 [ADR0028](decisions/0028-tagged-windows-installers.md)。

Windows Installer 维护模式可恢复已安装产品的上下文，不能仅凭命令行 `ALLUSERS` 或退出码判断是否改变范围。验收使用 `MsiEnumProductsExW` 核对当前用户 unmanaged 上下文，并检查实际载荷、登记及机器级目标；不读取或上传用户 SID。
