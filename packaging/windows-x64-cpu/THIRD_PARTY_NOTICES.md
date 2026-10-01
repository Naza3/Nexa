# 第三方组件与私有开发范围

本包用于 Nexa 内部开发验收，不表示 Nexa 项目已选择公开开源许可、提供正式发布授权或完成公开发行审查。未新增或接受任何许可协议；不据此推断用户拥有 Enterprise 授权。

## Rust 与原生组件

`licenses/index.json` 记录每份原文的组件、来源及 SHA-256。`licenses/rust-crates/` 包含实际 Windows 产品依赖图的 normal/build 闭包（保守保留过程宏等构建依赖的原文）；`licenses/rust-std/` 保留锁定 Rust 工具链提供的标准库版权和许可原文。`licenses/llama.cpp/` 保留固定 vendor 的 llama.cpp、nlohmann/json、cpp-httplib、hash 组件及 sheredom/subprocess.h 头文件内的原始 Unlicense 块。具体上游 commit 和包版本见 manifest。

## Microsoft Visual C++ Release x64 runtime

CRT DLL 来自 manifest 中实际 `vswhere` 所选 Visual Studio 安装根的 `VC/Redist/MSVC/<实际版本>/x64/Microsoft.VC143.CRT`。仅复制 PE 导入闭包确实需要的原文件；每个 DLL 的实际源、文件/产品版本、SHA-256 和 Microsoft Authenticode 签名结果在 `crt_sources` 中。不复制 Debug、debug_nonredist、onecore Debug 或预发行组件，不从 System32 取文件，不把编译器版本、工具集版本与 Redist 版本混为一谈。

- 官方 Visual Studio 2022 可再分发文件清单：https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution
- Visual Studio 2022 Community 条款原件（第 4 节 Distributable Code）：https://visualstudio.microsoft.com/wp-content/uploads/2021/11/Visual-Studio-2022-Community-License-EN.docx
- Visual Studio 许可入口：https://visualstudio.microsoft.com/license-terms/

上述来源和现有合法 Visual Studio 安装的适用条款用于本次标准 app-local 私有开发分发；这份说明不是替代微软条款的自创 EULA。程序构建不会安装 Redist、购买软件、接受新协议或保存新的授权状态。如果所选安装没有合法的 Release Redist 文件，构建直接失败并要求另外处理。

Windows 10 UCRT 及 OS/API-set 依赖由目标 Windows 提供，列在 manifest 的 OS 闭包中，未捆绑。组件自身的原许可及版权声明保持有效。
