# Tauri Windows 安装模板

模板基于 **Tauri CLI 2.12.1** 的固定提交
[`30da1fd6e17de6107ecc850c95dfb16b5729f2dd`](https://github.com/tauri-apps/tauri/tree/30da1fd6e17de6107ecc850c95dfb16b5729f2dd)。
上游采用 `Apache-2.0 OR MIT` 双许可证；原件见 [Tauri MIT](licenses/tauri-2.12.1/LICENSE-MIT)、[Tauri Apache](licenses/tauri-2.12.1/LICENSE-APACHE-2.0) 及包含 Cargo-Bundle 开发者声明的 [bundler MIT](licenses/tauri-2.12.1/bundler-License-MIT.md)/[bundler Apache](licenses/tauri-2.12.1/bundler-License-Apache.md)。NSIS/WiX/插件原件和来源摘要由 [sources.json](licenses/sources.json) 管理，随桌面包现有许可证库存无损整合。

| 本地文件 | 上游来源 |
| --- | --- |
| `installer.nsi` | `crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi` |
| `main.wxs` | `crates/tauri-bundler/src/bundle/windows/msi/main.wxs` |

这仍使用 Tauri 的 NSIS/WiX 打包、资源清单、界面和安装登记，少量模板适配用于保持 Nexa 已有约束：仅当前用户，固定 `%LOCALAPPDATA%\Programs\Nexa`，安装前拒绝运行中的 Nexa，不强制终止进程，卸载保留模型、设置、密钥和其它用户数据。NSIS 仅管理当前用户固定 `Programs\Nexa\Nexa.lnk` 开始菜单项，不创建或删除桌面快捷方式；去除开始菜单位置选择、自选目录、按名称扫描并卸载 MSI 的默认迁移、强制关闭程序及删除用户数据选项；WiX 保持既有 MSI 产品族和事务升级，并使用当前用户目录及保护检查。模板更新需重新比对固定上游，而非直接替换掉这些边界。

## 两种格式及旧版本

旧 `Setup.exe` 内嵌的是 MSI；它不是 NSIS。已有旧 Setup/MSI 的用户应下载新的 **MSI** 原位更新。同一安装目录不能同时由两套卸载登记管理，所以新的安装器拒绝直接跨 MSI/NSIS 覆盖。需要换格式时，先通过 Windows 应用设置卸载 Nexa，再安装另一种格式；模型和用户数据保留。

NSIS 重新安装同版本或升级版本时，先检查现有登记指向固定目录的 `uninstall.exe`，再执行该卸载程序清理其拥有的旧文件，最后安装新文件。因此删除或改名的旧资源不会遗留。只执行已登记的同格式卸载器，不执行其它格式的卸载命令。降级、无法确定版本、缺失或不匹配的卸载器均停止。

自动启动由桌面应用显式设置，安装器不自动启用。NSIS 同版本/新版本替换使用 `/UPDATE` 保留现有设置；真正卸载时，仅当 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 的 `Nexa` 值严格等于带双引号的本安装目录桌面 EXE（无参数）时删除该值。若它指向另一份便携版或读取失败，保留，不删除整个 Run 注册表项。

**NSIS 替换不是原子事务。** 若旧程序卸载后新安装失败，不会自动恢复旧程序；用户数据仍保留，可修复原因后重试安装。MSI 的事务升级与 NSIS 的替换流程须分开验证。

## NSIS 构建接口

生成配置的 `bundle.windows.nsis.installerHooks` 指向一个生成的 `.nsh` 文件：

```nsh
!define NEXA_CHECK_SOURCE "C:\absolute\build\tauri-installers\nexa-install-check.exe"
```

路径由生成器执行 NSIS 字符串转义。检查程序嵌入安装器和卸载器，解压至各自的 `$PLUGINSDIR`；不会写入应用或用户数据目录。其命令行契约为：

- `/nsis`：检查固定安装目录与运行中进程，同时拒绝该 MSI 产品族的现有登记。
- `/check`：检查固定安装目录与运行中进程，用于同格式卸载。
- 旧单参数调用保持原有返回值：`0` 允许继续，`1603` 拒绝路径/进程状态，`1150` 表示系统版本不支持，`1638` 表示检测到 MSI；参数/内存/MSI 查询错误沿用 Win32 返回值。
- 当前 NSIS 模板追加 `/diagnostic`，只在此模式下将检查失败映射到固定内部原因码：`20001` 系统版本、`20002` 用户目录查询或拼接失败、`20003` 安装/开始菜单路径无法验证、`20004` 进程仍在运行或无法验证状态、`20005` MSI 登记查询失败。成功与 MSI 冲突仍为 `0`、`1638`，未知参数拒绝为 `87`。这些内部原因码不是安装器对外退出码。
- NSIS 根据已知原因给出固定中英双语提示和不含路径的详情记录；无法启动 helper 与未知失败分别使用明确的启动失败和通用提示。对外仍返回 `1603`（检查失败）或 `1638`（MSI 冲突）。路径/进程检查采用失败关闭策略，不能把查询失败断言为链接存在或进程正在运行。
- MSI 使用独立的 `NexaGuard` DLL custom action 和 OS helper，本次诊断模式不改变其返回码、事务与回滚合同。

模板也独立核对实际 `$INSTDIR`，不能只依赖检查程序针对固定目录的验证；`/D` 和卸载 `_?=` 均不可扩大安装范围。静默 `/S`、被动 `/P`、GUI 安装与卸载使用相同检查；检查失败不会强杀程序。

`scripts/test_tauri_nsis.py` 验证模板的安全与调用顺序合同；`scripts/test_install_check_diagnostics.py` 用宿主 C 编译器和模拟 Win32 调用执行 helper 的分支、参数和旧返回值合同，并核对 NSIS 原因映射。它不代替 Tauri 渲染、真实 NSIS 编译以及 Windows 上的安装、同版本替换、忙进程拒绝、跨格式拒绝和卸载数据保留测试。
