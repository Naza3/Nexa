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


### 原生 MSI 宿主验证补充

第三轮原生日志已定位到旧 DLL OS guard 的拒绝窗口，尚未区分 API 返回失败还是兼容版本信息。OS 门禁采用和 Setup 相同 manifest/谓词的独立只读 EXE，并作为同步、检查返回值的 [MSI Type 2](https://learn.microsoft.com/en-us/windows/win32/msi/custom-action-type-2) 动作置于事务前。DLL 继续只读验证当前用户目录、路径和进程。早期真实 msiexec 私有测试只运行这些门禁，不启动安装事务或登记应用，失败优先暴露于耗时编译之前；实际应用安装/修复/升级/回滚门槛不因此放宽。

OS API 契约参考：[VerifyVersionInfoW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-verifyversioninfow)、[EXE supportedOS manifest](https://learn.microsoft.com/en-us/windows/win32/sysinfo/targeting-your-application-at-windows-8-1)。共享谓词检查主/次版本及两个 service-pack 字段的最低值，任何 API 错误或不满足版本要求均拒绝；文档已标记该查询依赖 manifest，不能把 DLL 宿主结果等同独立 EXE。


第四轮原生证据确认旧文件查询在 MSI 中返回 6.3.20348 而被阈值拒绝，新独立 OS 门禁与默认安装/修复已过。维护模式下 Windows Installer 可恢复既有每用户上下文，测试改为通过 [MsiEnumProductsExW](https://learn.microsoft.com/en-us/windows/win32/api/msi/nf-msi-msienumproductsexw) 及载荷/目录不变量核对实际结果；初装机器级/跨目录请求仍须失败，生产 guard 不变。维护返回码与上下文事实分开，剩余完整生命周期继续由新原生 CI 判定。
