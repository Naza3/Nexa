# Windows MSI 与 Setup 安装契约

## 用户选择

同一个 `vMAJOR.MINOR.PATCH` Release 提供三种格式：

- `Nexa-<版本>-windows-x64-portable.zip`：完整解压后直接运行
- `Nexa-<版本>-windows-x64-setup.msi`：原生 Windows Installer 包，默认使用系统自带 Basic UI，支持安装、修复和卸载
- `Nexa-<版本>-windows-x64-setup.exe`：带“安装/升级、修复此版本、移除此版本”选择、确认页、进度和结果页的离线向导，内部携带上面同一份 MSI

EXE 不是另一套安装器。用任一安装格式安装，另一格式操作的是同一 MSI 产品；同一版本的文件、组件、注册表登记和卸载身份一致。不同版本的“修复/卸载”请使用与已安装版本相同的安装包；旧版安装包不能降级新版。便携版不登记 MSI 产品，也不会被安装器移动、迁移或删除。

所有格式目前都没有 Nexa 代码签名。SHA-256 用于验证字节一致性，不证明发布者身份；仅从可信的仓库 Release/对应 Actions 获取并核对摘要。Windows/SmartScreen 可能显示未知发布者提示；不要把绕过安全警告当成安装步骤。

## 安装位置、权限与数据

固定每用户安装到 `%LOCALAPPDATA%\Programs\Nexa`，不请求管理员权限、不提供机器级安装、不修改 PATH/防火墙/系统服务，不建立自启动任务。开始菜单只为当前用户创建 Nexa 快捷方式。

- 配置、密钥、登记元数据等仍由应用放在 `%LOCALAPPDATA%\Nexa`
- 应用已有默认模型位置是 EXE 邻近的 `models`；这个目录在每用户安装位置可写
- 升级、修复、卸载只管理清单列出的程序文件与安装器自身的 HKCU 登记/快捷方式
- 安装目录内用户添加的 `models`、`model`、其他非安装器文件，以及数据目录和外部模型全部保留
- 不做递归目录删除；仅删除已经变空的安装器目录。因此卸载后仍有模型的 Nexa 目录会保留
- 不自动迁移便携版模型或数据。已有数据结构的兼容边界仍以应用版本为准，升级前备份重要配置与登记记录

不要把安装器自带 EXE、DLL、manifest 或许可文件当作可修改的用户数据：显式修复会从相同安装源恢复这些安装器拥有的文件。

安装/升级/修复/卸载前，在 Nexa 中停止服务并关闭窗口。原生只读检查会识别安装目录内的桌面、runtime、worker 和下载进程，包括可解析的短路径别名；发现运行中或无法确认其状态时拒绝继续。安装器不发送强杀、不静默停止服务，也关闭 Windows Installer 自动重启应用的行为。检查安装目标、受管文件及开始菜单路径中的重解析点，拒绝跨目录重定向。范围比较只展开已经存在祖先的 Windows 长名，允许同一目标的 8.3 路径别名；缺失尾部原样保留，拒绝 `.`、`..` 等改写。检查与后续 Windows Installer 写入仍不是针对同一用户恶意并发改目录的安全隔离边界。

## 前置组件与系统边界

目标仍是 Windows 10 x64 / i5-8400 / 16GB，Windows 11 后续按真实结果验证。Windows Server 2022 runner 上通过不能代替该目标设备、干净机器、离线和长期运行验收。

Microsoft Edge WebView2 Evergreen Runtime 必须已经安装。安装器不捆绑、不下载、不安装 WebView2，不接受其新的许可条款；向导显示说明，应用沿用原有缺失诊断。缺失时由用户通过 [Microsoft 官方 WebView2 页面](https://developer.microsoft.com/microsoft-edge/webview2/) 获取并自行安装。CLI runtime 不依赖 WebView2。应用本身的 CPU 指令集、内存和模型许可要求保持不变。

Setup 与 MSI guard 由同一既有 MSVC/Windows SDK 构建，使用 Windows inbox API，没有动态或静态 CRT 依赖；构建通过 `dumpbin /dependents` 的受限白名单检查。MSI 保留完整便携版许可闭包和 aria2 对应源码，不添加另一组重复许可文件。没有引入新 WiX/NSIS 运行依赖、EULA 自动接受、付费工具或签名证书。

## 命令行

普通用户双击 Setup 可使用向导。自动化使用：

```powershell
.\Nexa-0.1.0-windows-x64-setup.exe /S
.\Nexa-0.1.0-windows-x64-setup.exe /S /repair
.\Nexa-0.1.0-windows-x64-setup.exe /S /uninstall
msiexec.exe /i Nexa-0.1.0-windows-x64-setup.msi /qn /norestart
msiexec.exe /fa Nexa-0.1.0-windows-x64-setup.msi /qn /norestart
msiexec.exe /x Nexa-0.1.0-windows-x64-setup.msi /qn /norestart
```

示例版本只是命令格式，不表示已创建这个 tag 或 Release。Setup 只接受 `/S` 及至多一个 `/install`、`/repair`、`/uninstall`，不把任意参数转交 msiexec。始终调用 Windows 系统目录中的 msiexec，禁止自动重启，返回其原始退出码：`0` 成功、`1602` 用户取消、`3010` 成功但 Windows 请求稍后重启，其他非零值为失败；非法参数返回 `87`。

向导执行过程中不能直接关掉向导丢弃结果。若需要取消，使用 Windows Installer 进度窗口的“取消”，等待其回滚及向导结果；静默执行没有交互式取消按钮。完成后临时 MSI 删除。Windows Installer 缓存不承诺保留修复所需 CAB，控制面板修复可能询问安装源；保留相同版本的 MSI 或重新运行该版本 Setup 并选择修复。不会从网络补取未知安装源。

## 身份、事务与构建

- 固定 UpgradeCode：`{85615F8B-FD70-53D9-86ED-4164379CBC40}`，只用于 Nexa 的 x64 每用户产品族
- ProductCode 由该产品族、范围和严格三段版本派生；组件 GUID 按该产品族、范围、架构、规范化相对路径派生
- PackageCode 每次创建随机生成。即使提交和载荷相同，重建也可能产生不同 MSI 字节；不可覆盖已发布 tag 资产
- 只接受项目所有版本源一致的稳定版本，不归一化预发布/build 后缀；MSI 三段字段范围为 255/255/65535。`255.255.65535` 虽是 MSI 字段极限，但当前升级回归需要一个更高的私有测试版本，因此该终端版本会被 CI 阻断，须先明确新的测试策略
- 新版本先通过只读检查，再在 `InstallInitialize` 后、生成文件操作前执行 `RemoveExistingProducts`；旧卸载和新安装在同一 Windows Installer 事务内。失败回滚由 Windows Installer 负责
- 不允许降级，不把同一语义版本重新构建当成新的正式版本。发布流程见 [tag 发布流程](windows-releases.md)

生产命令：

```powershell
python scripts/package_windows_msi.py --payload dist/desktop-windows --version <版本> --output dist/Nexa-<版本>-windows-x64-setup.msi --setup-output dist/Nexa-<版本>-windows-x64-setup.exe --report dist/windows-msi-build-report.json
python scripts/test_windows_msi_lifecycle.py --payload dist/desktop-windows --msi dist/Nexa-<版本>-windows-x64-setup.msi --setup dist/Nexa-<版本>-windows-x64-setup.exe --report dist/windows-msi-lifecycle-report.json
```

第一步复用已经验收的便携目录，不重新构建应用；检查完整清单/依赖/许可/同源源码，逐文件生成标准 MSI 表及 CAB，再读取表和嵌入 CAB 校验。Setup 的资源必须与最终 MSI 完全相同，执行前在内存及提取后核对同一 SHA-256。提取使用不可预测且仅所有者/SYSTEM 可访问的临时目录，保持只读锁直到 msiexec 结束。

第二步只允许一次性 GitHub Windows runner，且拒绝已有 Nexa 安装或数据目录。真实运行不带 `/q` 的默认 MSI 入口（`LIMITUI=1` 明确使用系统 Basic UI）、逐字节安装校验、MSI/EXE 修复、运行中拒绝、scope/path/junction 拒绝、失败升级回滚、升级、降级拒绝、卸载、模型/配置/密钥/外部文件保留、Setup 三动作向导的前进/后退/取消、真实 GUI 安装的 Apply→受保护进度→Finish，以及实际 MSI `3010` 的 EXE 返回传播。私有升级/故障/请求重启测试 MSI 不进入 Release。

`--check-toolchain --report <路径>` 仅提前编译原生辅助程序、检查 inbox DLL 依赖，不生成真实安装器、不执行安装，不能作为生命周期通过证据。表结构/外键/读回校验也不冒称 Windows SDK ICE 验证；当前报告明确记录 ICE 未运行。尚未执行的 Windows 测试不得因为测试脚本存在而写成通过。


## 失败诊断

生命周期检查在每个外部动作前后即时输出固定阶段名、预期/实际数值退出码，写入 `windows-msi-diagnostics.json`。240 秒单次安装期限不因 UI 等待而延长，也不强杀 Windows Installer；超时时提取本次进程树窗口的固定原因枚举、控件 ID、可见/启用状态，以及 MSI 日志中白名单动作和数值错误码。原窗口文本、命令行、配置/令牌、完整路径和原始 MSI 日志不进入该报告。

失败时工作流只上传严格 schema/来源校验后的独立诊断目录；未知字段、重复 JSON 键、非 JSON 数值、路径/自由文本、超界数据和输入/输出祖先的 reparse 均拒绝。缺失报告明确记录 missing，不标成通过；通过全量校验后才原子发布证据目录。

首轮 `51d2d50` 的 Windows CI 已实际完成辅助程序编译和真实 MSI/Setup 字节绑定，在生命周期极早的一次动作等候 240 秒后失败；因当时缺少阶段诊断，不能确定具体 MSI 模态框或动作。后续 Basic UI 契约明确化、目标长短路径归一化和失败诊断属于针对源码缺口的限定修正，不宣称已证实首轮根因；完整安装、修复、升级、回滚与用户数据保留仍须新 CI 实测。
