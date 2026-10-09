# ADR0007：T06 原生壳与桌面开发包边界

2026-10-09更新：启动库存仅校验声明产品文件，未声明内容不再扫描或拒绝；以下旧GGUF/下载残留例外规则由[ADR0041](0041-declared-payload-startup-validation.md)替代。模型准入和下载执行安全边界保持。

日期：2026-10-01。状态：已采用；Windows 构建、真实 UI 操作结果另记，不由本决策认证。

## 决定

1. Tauri 2 独立 workspace/锁；根 workspace 只接入纯 Rust desktop-bridge。Rust 壳图不依赖 engine-host、llama-adapter 或 runtime-worker。WebView 不做 HTTP、令牌读取或通用系统操作。
2. AppManifest 明确列出契约20命令（原15加外部目录5命令），唯一 capability 仅 main 本地窗口/WebView。没有通用 shell/fs/http/clipboard-read 插件权限。导航只允许打包本地 origin，禁止新窗口和远程资源；生产 CSP 不开放本机 API 地址。开发 localhost 只在 debug 编译可用，不作为生产后门。
3. 原生选模仅保留一个一次性 selection ID；路径由 native picker 提供，祖先 symlink/reparse/UNC/device 检查失败即拒绝。传给 bridge 的是原始盘符路径，避免 canonicalize 的 `\\?\` 前缀被既有模型安全检查拒绝；bridge 独立复验。ID 消费后失败不自动重放。
4. 关闭按钮、窗口 close event、app exit request 使用同一异步状态机；重复关闭合并。默认清理本 UI 请求后保留 runtime；选择同时退出后必须确认 shutdown/实例释放。失败保留窗口并提供重试或仅关 UI，不按 PID 猜测强杀。壳不持 Tauri sidecar、kill-on-drop 或 UI-owned Job。runtime 独立启动/回收由 bridge 负责。
5. 令牌只经现有私有文件检查后写入 native clipboard，WebView 只收到 copied 布尔值。没有 token getter 或 clipboard-read。界面必须提醒令牌进入系统剪贴板。
6. WebView 创建前调用实际 WebView2 version 检测；缺失给原生错误及微软官方入口。本轮使用已安装 Evergreen，不下载/安装，不 Fixed Runtime，不改 ACL/ExecutionPolicy/Defender。
7. 独立 `desktop-windows.zip` 嵌入前端 EXE，并完整保留同源 `runtime/` T05 包、manifest、SHA256SUMS 与许可。包的源码/根锁身份须匹配；桌面额外 PE/app-local CRT 按实际 import 闭包验证。UI/runtime/model 字节数分开记录，模型和用户数据不入包。自校验仅表示一致性，不冒充签名。
8. `--diagnose` 只报告实际安装的 WebView2 版本、桌面包身份与完整性，不建立 WebView、不初始化数据、不启动 runtime。真实 bridge harness与独立Windows 10原生UI操作分层保留，不能互相替代。

## 同级模型输入与启动诊断补充

在受控解压的bc43e0f3产品包旁加入[模型矩阵](../model-matrix.md)锁定的公开Qwen3-0.6B Q8_0后，本地工程复现确认旧启动校验将该未声明GGUF拒为`package_unlisted_file`，通用提示未说明具体原因。固定输入与正负例见[T06验证记录](../verification/2026-10-01-t06-desktop.md)。工程要求允许模型与程序同级放置，因此将发行包payload与解压后外置输入区分；该复现不等于新版本Windows运行验收。

桌面根目录及固定`model/`、`models/`目录中直接、未声明的普通`.gguf`可作为外置输入，大小写不敏感、祖先和文件均不得为symlink/reparse，只读四字节GGUF魔数。两个固定目录可为空，不能嵌套或放入其他未知文件。没有启动时整模型hash、结构认证、自动导入或加载。所有声明文件size/hash、manifest/SHA256SUMS、桌面/runtime来源不变；runtime/licenses等位置及未知DLL/EXE/脚本继续拒绝。打包仍要求精确payload且模型0bytes，不将用户输入纳入发行ZIP或源码。

启动错误使用闭合错误码和分类中文提示；`--diagnose`升schema 2，失败仍不给未验证的source身份，且不输出路径/内容/任意系统错误文本。wrapper与stager均严格检查诊断字段和码值，保留合法失败报告及非零结果。现有Windows桌面验收步骤增加实际EXE同级真实GGUF正例，以及附加DLL/改动manifest负例，测试后清理仅自有输入并核验payload恢复。该新增行为须由新源码Windows CI验证，不追溯为旧包已支持。

工程要求为设置可选择模型目录并直接使用已有GGUF、不再复制，自动按文件名命名。按[外部目录契约](../t06-model-directory-contract.md)新增原生folder picker与一次性选择、扫描/取消操作；支持任意受控本地只读目录，元数据和token继续留AppData，不自动迁移或删除文件。包外目录不属于包库存；选择包内位置只支持程序根/model/models，其他包内目录当场拒绝。包校验不读取用户配置决定例外。外部注册/加载核验及Windows运行时source guard由model-store/API负责，不由四字节启动检查冒充；关闭时必须等待本窗口扫描实际清理。

## Windows外部宿主Job边界补充

2026-10-01的固定Windows对照已观察：同一harness EXE和DETACHED_PROCESS/CREATE_NEW_PROCESS_GROUP条件下，请求BREAKAWAY时进程创建返回OS5；省略BREAKAWAY时父/子均在Job中，子进程正常退出且Ctrl+C在250ms观察窗内保持pending。两份报告hash及源码见[T06第四轮记录](../verification/2026-10-01-t06-desktop.md)。这支持当前宿主Job不允许显式脱离，不代表查询了其具体限制位，也不是生产runtime或UI验收。

据此固定生产启动为`DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`，移除BREAKAWAY请求；不增加失败后fallback，不改CLI信号处理、Job限制、权限或系统安全配置。Nexa保证自己不创建kill-on-UI-close的runtime Job，不以正常UI退出或句柄析构结束runtime；保留外部宿主已有containment。不承诺外部宿主整个Job、登录会话或系统终止后runtime仍存活，不尝试绕过它。runtime自身对子worker的T03 Job回收约束不变。

默认关闭自身UI后保留同一runtime，以及选择同时退出后的真实清理，仍必须由修复源码的完整Windows Release bridge生命周期和独立原生窗口操作分别验证。早期双策略探针继续保留作诊断，不替代最终产品门槛。

## 构建与许可

- 当前独立图锁定 tauri 2.12.1、tauri-build 2.7.1、rfd 0.17.2、clipboard-win 5.4.1；JS API/CLI 2.12.1。精确图在各自锁文件，不共用或链接根 Cargo.lock
- 采用 `CARGO_CFG_TARGET_OS` 判断 build-script 目标；不能用 build-script 宿主 `cfg(windows)` 误跳过跨编译的 ACL 生成
- registry 缺失许可原文的七个 crate 通过 `.cargo_vcs_info` 精确 revision 补充，打包再次校验 hash/revision。selectors 的上游也缺 MPL 副本，使用 Mozilla 许可证维护方原文并保留源码获取位置
- WebView2 COM wrapper 的 MIT 许可不替代原生 loader 许可。已将 `webview2-com-sys 0.39.1` 的 x64 `WebView2LoaderStatic.lib` 与微软 NuGet SDK 1.0.3800.47 中原文件逐字节比较，保存 SDK 原 LICENSE/NOTICE/nuspec 与整包、loader hash。仅静态 loader 进入 EXE，不捆绑整个 WebView2 Runtime

## 不包含

不引入 Telegram、服务安装、公网监听、模型市场、GPU、自动更新、持久化聊天或新凭据权限。T05 后期无开发工具/离线/长期稳定性条件不由桌面包新增结果自动变为通过。
