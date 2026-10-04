# T05 独立包验收器

`nexa-acceptance` 是预编译的独立 Rust 工具。与 `xtask api-smoke` 共用
`src/api_smoke.rs` 的独立 HTTP/JSON/SSE oracle 和既有客户端 proof 传输；
不另外复制一套接口成功条件，也不链接 llama 原生推理实现。
纯 Rust MSVC 程序仍可能需要 VC CRT。工具包和产品包各自携带经实际 PE
依赖闭包核实的运行库，不能互相补齐依赖。

## 目标用户运行

从同一次构建取得产品 ZIP、验收工具 ZIP 及分别提供的 SHA256。
通过可信渠道核对哈希，再分别解压；SHA256 只证明完整性，没有发布签名
时不声称证明来源。不要向产品目录添加文件，不需要安装 Rust、Python、
Visual Studio 或额外服务，不修改 ExecutionPolicy、杀毒或系统安全设置。

建议布局：

```text
验收目录/
  windows-x64-cpu/       产品 ZIP 内容
  acceptance-tools/     工具 ZIP 内容，含其自身必要 CRT
  report.json           运行后生成，留在本机
```

在命令提示符或 PowerShell 中运行：

```powershell
.\acceptance-tools\nexa-acceptance.exe --model "D:\模型 空格\Qwen3-0.6B-Q8_0.gguf" --out ".\report.json" --machine-role target
```

工具默认从自身目录的上一级寻找 `windows-x64-cpu`；如果两个 ZIP 解压在
其他位置，使用 `--package "D:\实际 产品目录"`。不依赖仓库 CWD。不要把验收
工具放进产品目录。`--machine-role` 可为 `target`、`ci`、`unknown`，默认
`unknown`；这是运行者声明而非机器身份认证。模型和报告路径必须显式提供。
断流循环默认 5 次；`--disconnect-cycles 1..50` 可显式设置，最终 Release CI 用
`--disconnect-cycles 50`。报告分别记录 requested、attempted、passed 次数。
报告不能覆盖模型、产品或工具包文件；只允许原子替换同类已知验收报告。
报告包含数字/状态/哈希，不包含令牌、模型完整路径、提示或生成正文。报告
不自动上传，由用户自行交回需要分享的报告。失败也尽量保留脱敏 JSON。

固定模型 SHA256：

```text
9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031
```

固定验证参数为 CPU、context 2048、threads 2、batch 128、gpu_layers 0。
工具只修改自己拥有的临时测试配置，不改变产品 generic 4096 默认值或用户
数据，也不通过降低参数制造通过。

## 覆盖与边界

1. 校验 schema、产品版本/commit、llama commit、CPU/Release/x64 和
   HTTP/worker/shim 协议身份；逐文件核对大小、SHA256、完整清单和 SHA256SUMS
2. 独立读取 AMD64 PE 普通及 delay-load imports，与构建时 dumpbin 清单比较，
   核对所有非系统依赖都在产品目录；拒绝 Debug CRT、路径逃逸、链接/reparse
   point、缺失、篡改、额外未声明文件和工具混入产品
3. 每次创建包含中文和空格的私有临时目录，从实际产品 CLI 执行 init、重复
   init 不重置、serve、在线模型导入/列表、固定参数 load、status、devices、
   unload；CLI 自己从同目录选择 worker
4. 共用 HTTP oracle 继续检查实际非流式/SSE/usage、FIFO/取消、五次断流后
   下一请求及协议拒绝情况，最后通过实际 CLI stop 统一关闭
5. 先验证同一 TCP 连接上的 HMAC proof 再发送 Bearer；不旁路身份、重连重放
   或把 HTTP 200 当推理成功
6. 每个产品子进程清空继承环境，只保留必要 Windows 系统目录搜索路径，并
   从空临时 CWD 启动。验收器 DLL 不参与产品搜索。清理只针对本工具创建的
   Child handle 和临时目录，不依据外部 PID 杀进程；关闭确认失败则报告失败
   并保留临时私有状态，不虚报无遗留

Release 配置来自已核对的发行 manifest/构建记录和拒绝 Debug CRT 的 PE 检查。
这些检查不构成对任意恶意重写二进制的来源认证；外部可信哈希仍然必要。

退出码 0 仅表示本次短包检查通过；1 为失败；2 为命令/不安全报告路径错误。
要求 Windows x64 实际运行。报告通过 RtlGetVersion 记录真实 build 和 workstation/
server 类型；Windows 10、Windows 11 与 Server CI 分开。PATH 中找不到开发工具
不证明其未安装；VC runtime 是否预装、完全离线条件默认 unknown。CI 有开发
工具，不能替代无开发工具的 Win10 目标机 A20。目标机声明、硬件/环境证据以及
长时间内存、精确 prefill/decode、完整 A01–A26 仍需独立记录。合理的远程不可证
项明确 skipped，实际 CLI 等发行必测项不能 skipped。

## 开发验证

构建和打包入口由 `xtask build --platform windows-x64 --backend cpu` 管理，
详见打包文档；目标机无需执行以下开发命令：

```text
cargo test --locked -p xtask
cargo check --locked -p xtask --all-targets --target x86_64-pc-windows-msvc
cargo build --locked --release --target x86_64-pc-windows-msvc -p xtask --bin nexa-acceptance
```

单测使用合成 PE，仅证明静态检查器逻辑，不能算实际包验收。显式 ignored
`real_product_lifecycle_uses_shared_oracle_and_reaps` 通过 `NEXA_ACCEPTANCE_CLI` 和
`NEXA_ACCEPTANCE_MODEL` 提供实际产品和固定 GGUF，`NEXA_ACCEPTANCE_REPORT` 可
指定脱敏开发报告。Linux 运行此测试只验证实现和生命周期，不算 Windows
Release 包或 A20。

已有服务的开发入口保留：

```text
xtask api-smoke --base-url http://127.0.0.1:PORT --data-dir PRIVATE_DIR --model MODEL_ID --out REPORT.json --cli ABSOLUTE_PRODUCT_CLI --release-acceptance true
```

这个入口仍会关闭指定测试服务，应只用于短命隔离实例。发行模式要求显式绝对
CLI 路径，缺失路径或实际 CLI 检查缺席即失败。默认开发模式仍允许未构建 CLI
时明确跳过，不能把这一结果写成发行验收通过。
