# 补充原许可来源

这里的原文仅补全注册表 crate 省略的许可证，不修改 Cargo 来源或已锁定源码。每份下载原文的来源、上游 `.cargo_vcs_info` revision 与 hash 在 `sources.json`。构建会再次校验，未知缺失仍失败。

selectors 0.38.0 声明 MPL-2.0，但发布 crate 与锁定上游树缺许可证副本（上游问题 https://github.com/servo/stylo/issues/317 ）。因此包含 Mozilla 作为许可证维护方发布的原文 https://www.mozilla.org/media/MPL/2.0/index.txt 。使用的未修改源码可从 https://crates.io/crates/selectors/0.38.0 与 https://github.com/servo/stylo/tree/572ecba2d1600e7c3d490586692a209faf703baa/selectors 获取。没有修改这些 MPL-covered 文件。

Microsoft WebView2 loader 的 MIT Rust wrapper 许可与 Microsoft SDK 原许可分开。`native-components.json` 记录原始 NuGet 1.0.3800.47 整包 hash，并已逐字节确认 crate 的 x64 WebView2LoaderStatic.lib 与 NuGet 中 build/native/x64/WebView2LoaderStatic.lib 一致。这里保留该 SDK 原 LICENSE.txt、NOTICE.txt 和包 nuspec；不把 loader 误认为捆绑整个 WebView2 Runtime。构建/打包不下载 SDK或安装 WebView2。
