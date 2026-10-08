# Tauri 安装器、托盘与开机启动验证

任务：Windows 安装器改为 Tauri MSI/NSIS、同版本覆盖、设置启用关闭到托盘与当前用户开机启动。产品版本保持 0.2.3。决定见 ADR0039/0040；本文件区分本地主机、交叉编译和原生运行。

## 已完成检查

- 前端完整 Vitest：46 文件、970 项通过；typecheck、lint、生产构建通过。
- Linux 桌面壳：42 项通过；desktop-bridge workbench：11 项通过。两 workspace fmt 检查通过。
- Python 严格编码模式完整 suite：359 项，354 通过、5 项既有平台跳过。包括新路径隔离、失败发布清理、模板、许可原件完整性和生命周期夹具测试。
- `.github/workflows/native-windows.yml` actionlint 通过。
- 最新三个原生安装 helper 使用 clang-cl/LLD 与 Windows MSVC SDK 实际编译、链接及 PE 导入白名单检查通过，没有执行 Windows 代码。
- Tauri CLI 2.12.1 + NSIS 3.11 完整生产打包入口在 Linux 实际运行成功。使用此前验证的 `0ecddad` 桌面载荷，解包后 28 个文件大小/hash 与输入相符，输入未改。探测 EXE SHA-256：`f62b7a27525d78cd5d7c2eafe68143ed53107b07d9f22d324e381580608e24ef`。这是新模板/打包流程检查，不是含新托盘功能的发行包。
- 最终 WiX 片段通过 WiX 3.14.1 官方 XSD 检查；未据此声称 candle/light 或 Windows 安装成功。

证据保存在云环境 `/workspace/onboarding/windows-installer-same-version/`；`frontend-final.log`、`python-final.log`、`shell-final.log`、`workbench-final.log`、`final-nsis-report.json` 和 `final-nsis-payload-verification.json` 为本次记录。helper 最终检查另见 `/tmp/nexa-helper-import-review-report.json`。

独立审查后已修复：生成物路径相互覆盖及污染输入、失败时部分发布、安装 helper 隐式编译参数和 DLL 导入约束、固定开始菜单范围、MSI 外部超长启动值误阻止卸载、Run 命令长度限制，以及迟到关闭请求覆盖托盘恢复的竞态。

## Windows 完整交叉构建

源码冻结后执行既有 `/workspace/onboarding/windows-cross/build.sh`，要求前端、原生库、桌面及 runtime 两 workspace 的 Windows Release 全目标 strict Clippy、四个 EXE 实际链接与源码/PE 身份检查全通过。执行结果在提交后补记；未完成前不以单独 Clippy 通过代替整个门槛。

## 尚待原生 Windows 验证

本轮没有推送、创建 tag 或触发 GitHub Actions。Windows 安装生命周期脚本已接入原有 CI，17 项门槛包含旧 MSI 迁移、两格式同版本替换和升级/降级、MSI 事务回滚、运行中拒绝、跨格式拒绝、用户数据与启动项保留/卸载清理。真实 MSI 编译、两格式安装/升级/卸载尚未运行。

用户 Windows 10 上的任务继续运行、托盘反复隐藏/恢复、明确退出时保存、注销再登录后的自动启动，以及 Windows 系统禁用启动项后的行为仍需实机确认。NSIS 更新不提供 MSI 式事务回滚；已有旧 Setup 应使用新 MSI 更新，切换格式需先卸载程序。

云环境已增加签名验证的独立 Debian NSIS/7zip 工具目录；安装脚本复跑通过，复用安装与启动说明已保存至环境配置草稿。保存草稿不等于发布快照，后续复用需在环境设置保存并发布。
