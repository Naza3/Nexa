# 桌面第三方原许可与内部开发范围

本包用于 Nexa 私有开发验收，不是公开 Release。Nexa 项目根许可尚未选择，本文件不替代上游原文，也不宣称完成外部分发授权审查。

- `licenses/index.json` 与 `licenses/rust-crates/`：独立桌面 Cargo.lock 中实际 Windows normal/build 依赖闭包的原许可（保守包含过程宏等构建依赖），包括 Tauri、Wry、WebView2 COM/loader、rfd、clipboard-win 及 Rust 标准库
- `licenses/npm-index.json` 与 `licenses/npm/`：嵌入前端的实际已锁定 npm production 依赖许可原文，包括 React、React DOM、scheduler、Tauri JS API；包版本和 registry integrity 一并保留
- `runtime/`：完全保留匹配源版本的 T05 CPU 产品、依赖、manifest、许可与第三方声明。桌面层没有重新打散或重新链接推理库
- 桌面 EXE 新增 app-local CRT 仅取所选 Visual Studio 已有 Release x64 Redist 闭包，验证 Microsoft Authenticode 并记录实际 DLL 版本、来源与 hash。许可规则和微软原件入口沿用 `runtime/THIRD_PARTY_NOTICES.md`
- 本包不包含模型、模型许可接受步骤、WebView2 Runtime 安装器、固定 WebView2 分发、自动更新器或系统安装脚本；已安装 Evergreen 的管理和适用条款由其原安装负责

Windows inbox DLL 仅在 PE 闭包中列为 OS 依赖，不从 System32 复制。`manifest.json` 的依赖图与每个原文件哈希供审核；未知 DLL 或缺原许可会使打包失败。
