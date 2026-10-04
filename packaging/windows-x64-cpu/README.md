# Nexa Windows x64 CPU 私有开发验收包

本包仅供已授权的内部开发和验收，不是公开发布。首要目标系统是 Windows 10 x64；Windows 11 后续验证。Windows Server 2022 CI 构建/测试不能代替 Windows 10、无开发工具独立机器或 i5-8400/16GB 实机验收。实际结果以与 manifest 精确对应的独立验收报告为准。

## 包内内容与边界

- `ai-runtime.exe`：本机管理 CLI/API，不链接 llama 原生库
- `ai-runtime-worker.exe`：CPU 推理 worker，必须和 CLI 在同一目录；不从当前目录或 PATH 搜索
- 实际导入闭包所需的 Microsoft Release x64 CRT DLL，均来自构建时所选 Visual Studio 的合法 VC/Redist/MSVC 源，保持未修改
- `manifest.json`、`SHA256SUMS`：精确构建身份、依赖闭包、DLL 来源/版本/签名、文件 hash/大小及许可信息
- `licenses/`、`THIRD_PARTY_NOTICES.md`：依赖许可原文及来源

Windows 10 UCRT 和 Windows 系统 DLL 由操作系统提供，不从 System32 复制进包。无需 Rust、Cargo、Python、CMake、Visual Studio 或 WebView 才能运行本包。CPU 至少需要 manifest 中的实际指令集；当前固定原生配置包含 AVX2/FMA/F16C 等，不能视为所有 x64 CPU 均兼容。无模型、UI、PDB、开发测试程序、用户数据、日志或真实令牌。模型文件独立取得并核对其许可、SHA-256。

`config.example.toml` 明确使用已测短基线：CPU / context 2048 / threads 2 / batch 128 / gpu_layers 0。它不改变程序默认值，也不会自动初始化凭据。

## 手动启动（PowerShell）

1. 完整解压到自选目录，两个 EXE 和所有随附 DLL 必须保持相邻；不要单独复制 EXE
2. 先核对下载的 ZIP 与旁边 `.sha256`，再核对包内 `SHA256SUMS`。可用 Windows PowerShell `Get-FileHash -Algorithm SHA256`
3. 为本次测试指定包外临时数据目录，例如 `$data = Join-Path $env:TEMP 'Nexa private test'`
4. 显式运行 `& '.\ai-runtime.exe' --data-dir $data init`。它为这个数据目录创建测试凭据，请勿打印或分享令牌
5. 将配置示例复制为 `$data\config.toml`；用 `& '.\ai-runtime.exe' --data-dir $data models import --id qa-small --file <独立GGUF文件>` 导入。以 CLI `--help` 为准，不把模型放进安装目录
6. 在一个 PowerShell 窗口运行 `& '.\ai-runtime.exe' --data-dir $data serve`，另一个窗口执行 `status`、`load qa-small --backend cpu --context 2048 --threads 2 --batch 128`；文本生成由 HTTP `/v1/chat/completions` 或独立验收器验证，当前 CLI 没有 chat 命令
7. 完成后运行 `& '.\ai-runtime.exe' --data-dir $data stop`；确认退出后再删除本次测试数据目录，不删除已有用户数据

已实现的管理命令可以先用 `ai-runtime.exe --help` 核对。HTTP 仅监听回环地址、需要本地令牌；不开放公网、不自动下载模型或注册常驻服务。

## 独立验收工具

另一个 `acceptance-tools.zip` 包含 `nexa-acceptance.exe` 及它自身所需的 CRT 和许可，不属于产品包。工具接受 `--package <本包目录> --model <外部固定GGUF> --out <报告路径>`。它核对完整性与 PE 依赖、从带中文空格的独立临时目录启动包内 CLI、清理开发工具 PATH，并仅使用临时数据/凭据进行真实模型检查。工具成功不等于已经完成所有 A01–A26、Windows 10/11 或目标设备性能验收。

本包没有更新器；更新时先正常 stop，保留包外用户数据，再整体替换程序目录。校验失败、缺少 DLL、未知依赖、CPU 指令不兼容或安全软件阻止时不要绕过警告，也不要从任意 DLL 下载站补文件。
