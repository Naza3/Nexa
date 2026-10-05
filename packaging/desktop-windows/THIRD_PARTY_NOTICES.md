# 桌面第三方原许可与内部开发范围

本包用于 Nexa 私有开发验收，不是公开 Release。Nexa 项目根许可尚未选择，本文件不替代上游原文，也不宣称完成外部分发授权审查。

- `licenses/index.json`：每份原许可的原始路径、组件/版本/来源映射、SHA256、原始字节数及存储位置。原 Cargo/npm 索引也作为完整原文保留在 `licenses/THIRD_PARTY_LICENSES.txt`，其中的旧路径是原文标识，不是本包中额外的文件
- `licenses/THIRD_PARTY_LICENSES.txt`：独立桌面 Cargo.lock 中实际 Windows normal/build 依赖闭包的逐字节原许可（保守包含过程宏等构建依赖），包括 Tauri、Wry、WebView2 COM/loader、rfd、clipboard-win；同时保留嵌入前端的 React、React DOM、scheduler、Tauri JS API 等实际已锁定 npm production 依赖原文、包版本和 registry integrity。重复许可也按各自组件完整保留
- `licenses/COPYRIGHT.html`：锁定 Rust 工具链的标准库版权 HTML 原件；其余标准库原许可收于上述文本，不转换换行、不删版权或 NOTICE
- `runtime/`：完全保留匹配源版本的 T05 CPU 产品、依赖、manifest、许可与第三方声明。桌面层没有重新打散或重新链接推理库
- 桌面 EXE 新增 app-local CRT 仅取所选 Visual Studio 已有 Release x64 Redist 闭包，验证 Microsoft Authenticode 并记录实际 DLL 版本、来源与 hash。许可规则和微软原件入口沿用 `runtime/THIRD_PARTY_NOTICES.md`
- 本包不包含模型、模型许可接受步骤、WebView2 Runtime 安装器、固定 WebView2 分发、自动更新器或系统安装脚本；已安装 Evergreen 的管理和适用条款由其原安装负责

Windows inbox DLL 仅在 PE 闭包中列为 OS 依赖，不从 System32 复制。`manifest.json` 的依赖图与每个原文件哈希供审核；未知 DLL 或缺原许可会使打包失败。

完整解压后的桌面目录（包含 `runtime/` 和 `download/`）至多十份许可相关文件：桌面本层四份、可独立使用的 runtime 四份、下载组件两份。GPL aria2 对应源码归档原样保留；不为减少文件数删除归档内源码、版权或许可。许可整合仅改变展示布局，不代表取得额外分发权。

交叉构建若有 Microsoft DOCX/PDF 等非文本许可原件，仍以原格式单独保存在 `licenses/ORIGINAL-<原文件名>`；这时本说明的完整原字节也收入文本索引，省去独立说明文件。索引中的原路径始终保留，原件不转码，runtime不依赖外层文件。
