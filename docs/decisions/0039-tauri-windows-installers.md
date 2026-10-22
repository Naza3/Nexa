# ADR0039：Tauri 官方 MSI/NSIS 与同版本覆盖

状态：已决定并实施源码；本轮原生 Windows 构建、生命周期与用户设备验收待验证

## 需求与范围

2026-10-08 用户报告同版本 `Nexa-0.2.3-windows-x64-setup` 无法覆盖已安装版本，并明确选择改用 Tauri 官方 MSI（WiX）和 NSIS EXE。旧实现的 ProductCode 只由版本派生，每次构建的 PackageCode 不同，而升级表只匹配更低版本，因此同版本重新构建不进入既有升级路径。

本决定替代 [ADR0028](0028-tagged-windows-installers.md) 中自建 MSI、EXE 嵌入同一 MSI 及不采用 WiX/NSIS 的工具选择。继续保持同源便携载荷、每用户固定位置、用户数据保留、原生验证和 tag 发行边界；不移动旧 tag、不替换已发布附件，也不把本次实现自动解释为推送或发布授权。

## 打包决定

- 正式入口改为 `scripts/package_tauri_windows.py`，调用 npm 锁定的 Tauri CLI `2.12.1`。模板基于 Tauri 提交 `30da1fd6e17de6107ecc850c95dfb16b5729f2dd`，使用官方 WiX/NSIS 生成器与标准界面，保留明确的 Nexa 目录、数据和进程检查适配。
- MSI 使用 Tauri 锁定的 WiX `3.14.1`（MS-RL）；NSIS 使用 `3.11`。不采用 WiX 7，不新增收费工具、签名证书或框架账户。上游模板及工具/插件许可原件和来源限制纳入既有桌面许可证库存，见[模板与许可说明](../../packaging/tauri/windows/README.md)。
- 便携目录是唯一应用载荷。生成隔离的临时 Cargo 元数据工程，只复制已验证的主 EXE，运行 `tauri bundle --no-binary-patching --no-sign`，不重编应用；其他资源按原相对路径进入安装器。构建前后与实际安装均核对应用文件字节，安装器自身的界面/卸载器另行区分。
- MSI 资源以标准 WiX fragment 生成，每用户组件使用 HKCU keypath，仅清理已空目录；主模板沿用标准安装/维护 UI，不恢复自研 Setup。现有原生 OS/路径/进程 guard 保留，旧自建 MSI 代码只用于私有旧身份迁移 fixture。

## 升级与迁移决定

MSI 保持 `{85615F8B-FD70-53D9-86ED-4164379CBC40}` UpgradeCode，采用 WiX 每包新的 ProductCode/PackageCode 和 `AllowSameVersionUpgrades`，关闭较低三段版本降级。`RemoveExistingProducts` 保持在 `InstallInitialize` 后，使旧卸载与新安装属于同一 Windows Installer 事务；失败回滚由 Windows Installer 处理。

因此旧 MSI 或旧内嵌 MSI 的 Setup 用户，应使用新 MSI 原位更新，包含旧 `0.2.3` 到新 `0.2.3` 构建。同一原始 MSI 的重复运行提供标准维护/修复。同版本新构建没有时间顺序，允许替换为选定构建，但不代表允许覆盖 tag 资产。

新 EXE 是 NSIS，不再与 MSI 共用产品登记。NSIS 在原包重装、同版本替换及升级前，验证既有同格式卸载器与固定路径，执行旧卸载器移除其清单文件，再安装新载荷。这样能移除新清单已删除或改名的旧文件；旧版本未知、卸载器缺失或登记不匹配则停止。

**NSIS 替换不是原子事务，也不承诺自动回滚旧程序。** 旧程序卸载后新安装失败时，用户数据保留，但需修复原因后重新安装。不能用 MSI 回滚测试为 NSIS 作相同承诺。

MSI/NSIS 双向拒绝直接跨格式覆盖，不自动执行另一格式的卸载命令。需要切换时用户先通过 Windows 应用设置卸载程序，再安装另一格式，模型/配置保留。旧发布安装器不追溯改变，仍须用来源提交与 SHA 区分相同版本/文件名的新旧包。

## 权限、数据与开机启动

安装固定为当前用户 `%LOCALAPPDATA%\Programs\Nexa`，开始菜单固定为当前用户 Nexa 项，不请求提升，不写 PATH、防火墙或系统服务，不提供用户数据清除选项。所有模型、配置、令牌、外部目录及非受管文件保留；安装前若发现桌面/runtime/worker/下载进程运行或状态不明则拒绝，不强停进程。

用户后来明确要求的开机启动由桌面应用单独控制，安装器不自动启用。`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 的 `Nexa` REG_SZ 只记录带双引号、无参数的当前桌面 EXE。更新保留该选择；真正卸载只删除匹配本固定安装位置的值，不删除其他副本的值或整个 Run 项。MSI 使用 commit action 且排除 `UPGRADINGPRODUCTCODE`，避免在失败更新中提前清理；NSIS `/UPDATE` 保留。应用侧托盘与启动设置另见 [ADR0040](0040-desktop-tray-and-autostart.md)。

## 验证与发行后果

`test_tauri_windows_lifecycle.py` 只允许空的一次性 GitHub Windows runner，真实验证旧 MSI 迁移、两格式同版本变更载荷的修改/新增/删除、MSI 写入后故障回滚、修复/重复安装、高版本升级与降级拒绝、进程占用、跨格式拒绝、卸载和用户数据/启动项保留。私有 fixture 不进 Release；失败诊断继续只允许固定阶段、动作和数值，不上传原始日志或用户路径。

发行报告沿用旧 `windows-msi-*.json` 文件名，但来源绑定和严格检查集合改为 Tauri 双格式。既有自研 Setup 的成功报告不满足新门禁。三格式应用字节相同不等于两个安装器字节或产品身份相同；重建后的新 GUID/安装器字节仍受原有 Release 防覆盖规则约束。

本轮原生 Windows 尚未执行。单元测试、XSD、辅助程序交叉编译、NSIS 交叉打包、原生 Windows 安装和目标机 GUI 分别记录；完整本地 Windows 交叉门槛通过后，只有得到推送授权才触发远端原生流水线。用户操作与正式命令见[安装契约](../windows-installers.md)，发行条件见[发行流程](../windows-releases.md)。
