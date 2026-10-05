# ADR0028：tag 同源三格式 Windows 发布

状态：实施；原生 Windows 生命周期结果以精确提交的 CI 报告为准

## 需求与决定

2026-10-05 用户先要求 GitHub tag 自动构建便携版和 MSI，随后补充 Setup 安装包。沿用公开仓库标准 GitHub runner，提交内的版本一致性校验和原有原生真实模型、下载组件、完整包验收通过后，tag 才能发布。

已有完整桌面目录是唯一分发载荷。便携 ZIP、每用户 MSI、向导 Setup.exe 三种格式复用相同已验证字节。EXE 只嵌入并启动同一 MSI，安装/修复/卸载及升级事务都归 Windows Installer。MSI 的原生 guard 只读检查目标和运行状态，不关闭进程、不修改用户配置/模型、不过问模型许可接受。

固定安装根为 `%LOCALAPPDATA%\Programs\Nexa`，保持应用 EXE 邻近默认模型目录可写；配置/令牌数据仍在 `%LOCALAPPDATA%\Nexa`。只删除安装器拥有的清单文件和已空目录，所有用户数据、模型与外部文件保留。产品族、版本和组件身份策略见 [安装器契约](../windows-installers.md)。

不使用 WiX 7：其当前版本要求显式 EULA，可能涉及维护费用，本轮没有相应接受/付费授权。也不为绕开该条款选择已经停止公开安全维护的旧 WiX。采用既有 Windows MSI API、MakeCab、MSVC 和 Windows SDK，没有新增第三方安装器框架、下载步骤或证书。向导和 guard 仅链接系统库，MSI 应用载荷的既有许可/对应源码闭包保持完整。

## 约束与后果

- 仅稳定三段版本；tag 不代写代码版本、不发明发布版本。正式 tag/Release 的产生由用户决定
- 同一个 tag 的 Release 内容不可悄悄覆盖。只重跑发布步骤可复用已有资产；全量重建会得到新 PackageCode/可能新字节，不能冒充原资产
- 未签名是公开事实，哈希不是签名；WebView2 仍由用户从官方来源安装，不在安装时静默联网
- 表级生成/读回、本机安装生命周期和 Win10 目标设备验证分层记录。Windows Server CI 不替代目标设备验收，ICE 未运行不冒称通过
- 发布细节见 [tag 发布流程](../windows-releases.md)，用户使用与安全边界见 [安装器契约](../windows-installers.md)

## 官方技术依据

- [Windows Installer API](https://learn.microsoft.com/en-us/windows/win32/msi/installer-function-reference)
- [升级表版本比较](https://learn.microsoft.com/en-us/windows/win32/msi/upgrade-table)
- [RemoveExistingProducts 事务顺序](https://learn.microsoft.com/en-us/windows/win32/msi/removeexistingproducts-action)
- [每用户不提升的 Summary 位](https://learn.microsoft.com/en-us/windows/win32/msi/word-count-summary)
- [LIMITUI 与系统 Basic UI](https://learn.microsoft.com/en-us/windows/win32/msi/limitui)
- [完整 UI 的执行序列](https://learn.microsoft.com/en-us/windows/win32/msi/installuisequence-table)
- [Windows 10 的 MSI VersionNT 仍为 603](https://learn.microsoft.com/en-us/troubleshoot/windows-client/application-management/versionnt-value-for-windows-10-server)
- [WiX 维护费与显式 EULA](https://docs.firegiant.com/wix/osmf/)
