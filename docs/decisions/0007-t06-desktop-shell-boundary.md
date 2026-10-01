# ADR0007：T06 原生壳与桌面开发包边界

日期：2026-10-01。状态：已采用；Windows 构建、真实 UI 操作结果另记，不由本决策认证。

## 决定

1. Tauri 2 独立 workspace/锁；根 workspace 只接入纯 Rust desktop-bridge。Rust 壳图不依赖 engine-host、llama-adapter 或 runtime-worker。WebView 不做 HTTP、令牌读取或通用系统操作。
2. AppManifest 明确列出契约 15 命令，唯一 capability 仅 main 本地窗口/WebView。没有通用 shell/fs/http/clipboard-read 插件权限。导航只允许打包本地 origin，禁止新窗口和远程资源；生产 CSP 不开放本机 API 地址。开发 localhost 只在 debug 编译可用，不作为生产后门。
3. 原生选模仅保留一个一次性 selection ID；路径由 native picker 提供，祖先 symlink/reparse/UNC/device 检查失败即拒绝。传给 bridge 的是原始盘符路径，避免 canonicalize 的 `\\?\` 前缀被既有模型安全检查拒绝；bridge 独立复验。ID 消费后失败不自动重放。
4. 关闭按钮、窗口 close event、app exit request 使用同一异步状态机；重复关闭合并。默认清理本 UI 请求后保留 runtime；选择同时退出后必须确认 shutdown/实例释放。失败保留窗口并提供重试或仅关 UI，不按 PID 猜测强杀。壳不持 Tauri sidecar、kill-on-drop 或 UI-owned Job。runtime 独立启动/回收由 bridge 负责。
5. 令牌只经现有私有文件检查后写入 native clipboard，WebView 只收到 copied 布尔值。没有 token getter 或 clipboard-read。界面必须提醒令牌进入系统剪贴板。
6. WebView 创建前调用实际 WebView2 version 检测；缺失给原生错误及微软官方入口。本轮使用已安装 Evergreen，不下载/安装，不 Fixed Runtime，不改 ACL/ExecutionPolicy/Defender。
7. 独立 `desktop-windows.zip` 嵌入前端 EXE，并完整保留同源 `runtime/` T05 包、manifest、SHA256SUMS 与许可。包的源码/根锁身份须匹配；桌面额外 PE/app-local CRT 按实际 import 闭包验证。UI/runtime/model 字节数分开记录，模型和用户数据不入包。自校验仅表示一致性，不冒充签名。
8. `--diagnose` 只报告实际安装的 WebView2 版本、桌面包身份与完整性，不建立 WebView、不初始化数据、不启动 runtime。真实 bridge harness、实际原生窗口和用户 Win10 UI 操作分层保留，不能互相替代。

## 构建与许可

- 当前独立图锁定 tauri 2.12.1、tauri-build 2.7.1、rfd 0.17.2、clipboard-win 5.4.1；JS API/CLI 2.12.1。精确图在各自锁文件，不共用或链接根 Cargo.lock
- 采用 `CARGO_CFG_TARGET_OS` 判断 build-script 目标；不能用 build-script 宿主 `cfg(windows)` 误跳过跨编译的 ACL 生成
- registry 缺失许可原文的七个 crate 通过 `.cargo_vcs_info` 精确 revision 补充，打包再次校验 hash/revision。selectors 的上游也缺 MPL 副本，使用 Mozilla 许可证维护方原文并保留源码获取位置
- WebView2 COM wrapper 的 MIT 许可不替代原生 loader 许可。已将 `webview2-com-sys 0.39.1` 的 x64 `WebView2LoaderStatic.lib` 与微软 NuGet SDK 1.0.3800.47 中原文件逐字节比较，保存 SDK 原 LICENSE/NOTICE/nuspec 与整包、loader hash。仅静态 loader 进入 EXE，不捆绑整个 WebView2 Runtime

## 不包含

不引入 Telegram、服务安装、公网监听、模型市场、GPU、自动更新、持久化聊天或新凭据权限。T05 后期无开发工具/离线/长期稳定性条件不由桌面包新增结果自动变为通过。
