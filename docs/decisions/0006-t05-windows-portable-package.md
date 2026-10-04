# ADR 0006：T05 Windows CPU 便携包与独立验收

- 日期：2026-10-01
- 状态：已实现并通过6a7e9d0的固定Windows Server2022 Release CI；用户Windows10 build19044 / i5-8400短验也已通过；用户批准延后无开发工具、离线与长期稳定性验证，T05按当前缩定范围已完成；A20仍未验证，T06进入开发。证据逐项记入 [T05 验证](../verification/2026-10-01-t05-windows-package.md)
- 范围：已授权开发分支上的私有内部包、独立验收工具和现有 Windows CI；无 GitHub Release、合并、部署、远控、系统服务或长期真实凭据初始化

## 1. 产物与构建边界

`cargo run --locked -p xtask -- build --platform windows-x64 --backend cpu` 只支持原生 Windows x64 MSVC 构建主机。它委托 `scripts/package_windows.py`，不自行下载安装器、模型或新原生代码。复用 `build/native-release` 的固定 CMake Release 原生树；重新配置并验证同一 VS 实例、x64、CPU、静态库与 `/MD`，仅增量检查 `air_llama`，不增加另一套完整 native workflow。编译身份用 CMake 为每个配置生成的精确 archive 路径验证，不递归猜同名 `.lib`。

Rust 产品使用独立 `build/windows-x64-cpu/cargo/x86_64-pc-windows-msvc/release`，明确 `--locked --release --target x86_64-pc-windows-msvc`。先用不存在的 `AIR_NATIVE_DIR` 构建 `ai-runtime.exe` 并检查 normal 依赖图，随后以可信原生树构建 `ai-runtime-worker.exe`。管理进程保持不链接 llama。构建、PE 检查、许可与完整性全部成功后才替换最终目录；失败不得留下看似完整的新包。

- `dist/windows-x64-cpu/`：两个产品 EXE、各自实际 PE 导入闭包所需 app-local DLL、配置示例、使用说明、manifest、SHA256SUMS 和许可
- `dist/windows-x64-cpu.zip`、`.zip.sha256`：ZIP 带 `windows-x64-cpu/` 顶层目录
- `dist/acceptance-tools/`、同名 ZIP/hash：单独 `nexa-acceptance.exe` 及它自身的 DLL/许可/完整性清单；产品包内不混入验证器
- PDB 独立保存，存在时单独上传；模型、临时数据/令牌、用户正文、构建缓存与原始本地路径不进入两份包

manifest 记录精确项目/llama commit、工作树状态、Cargo.lock hash、工具版本、Release/CPU/目标身份、文件大小/hash、真实 DLL 来源相对路径/版本/签名与依赖分类。SHA256SUMS 覆盖 manifest 与其他包文件，manifest 的 files 不递归包含自身或 SHA256SUMS。SHA-256 证明字节一致性，不等同发布者认证或代码签名。

## 2. 动态依赖与许可

产品与验收器分别从实际 PE 普通/延迟导入解析闭包；未知 DLL、Debug CRT、架构错误、缺失 app-local VC DLL、未声明/额外文件或 hash 不符即失败。运行成功不能替代静态闭包，因为 CI 全局安装的 VC runtime 可能掩盖缺包。Windows 系统 DLL/UCRT 由 OS 提供，不从 System32 复制。产品的 CWD 是自有空临时目录，PATH 只有系统目录，不能借验收工具目录补 DLL。

Microsoft CRT 只从本次 `vswhere` 对应 VS2022 的 `VCToolsRedistDir/x64/Microsoft.VC143.CRT` 复制所需未修改 Release 文件，保留实际 edition/版本、相对来源、签名、hash 与适用许可/REDIST 来源；不复制 Debug/Preview/全工具链，不安装新的 Redistributable。

现有标准 Redist app-local 开发分发未发现新的显式协议接受/购买/安装前置。不创造“接受许可证”flag，不假定用户有 Enterprise 授权，也不把 Enterprise/Professional 条款中的额外义务移植给 Community。参考 [Microsoft REDIST](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution) 与 [VS2022 Community 原文](https://visualstudio.microsoft.com/wp-content/uploads/2021/11/Visual-Studio-2022-Community-License-EN.docx)。实际安装源/许可不符或必须新安装/接受条款时停止相应步骤并报告。项目自有根 LICENSE 尚未选定，属于后续外部分发决策，不阻止本轮私有内部开发验收包。

## 3. 独立真实验收

`nexa-acceptance.exe --model MODEL.gguf --out REPORT.json [--package DIR] [--machine-role target|ci|unknown] [--disconnect-cycles 1..50]` 不依赖 Rust/Python/CMake。默认定位工具目录旁的 `../windows-x64-cpu`；CI 总是传显式 `--package`，工具在完整性校验后严格调用该目录的 `ai-runtime.exe` 和相邻 worker，再向 HTTP smoke 传明确 CLI 路径与严格发行模式，不因找不到 CLI 跳过。

只使用固定 Qwen3-0.6B Q8_0 SHA-256 `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`，CPU/context2048/threads2/batch128/gpu_layers0。无新模型源，不修改通用 context4096 默认。断流次数默认5、显式允许1..50；CI明确传50，不能因机器角色暗改参数。工具创建自己的中文/空格临时工作目录、数据目录、短命凭据和 loopback 端口，运行 CLI 初始化/重复初始化、服务启动/导入/模型管理/状态，以及 HTTP 真实生成、SSE、usage、取消、断连后恢复和关停。最终确认服务与 worker 回收，报告不保存正文、令牌或完整路径。

Exit 0 仅表示报告范围内短程包检查通过；1 为失败并尽量写安全报告；2 为参数错误。报告中的 skipped/unverified 不能改写为 pass。Windows 版本/build 取实际系统观测，`--machine-role` 为用户声明。PATH 中看不到 VS/Cargo 仅代表不可见，不能证明未安装开发工具、VC runtime 或实际离线。

## 4. CI 与阶段门槛

保留全部 T00–T04 原生、Rust、真实模型、DACL/Job、HTTP50 回归。在同一 job 末尾构建包并验证 ZIP 外部 hash，将两个 ZIP 解压至新的中文/空格目录，将已核验公开模型复制到自有中文/空格模型路径并复查相同hash，在非 repo 空 CWD 用解压后的独立工具验收解压后的产品。工具自己的受控模型存储根同样含中文/空格，真实load路径也必须覆盖。只有实际验收成功才上传产品与工具 ZIP/hash；单独上传符号及闭合允许列表中的脱敏证据，不创建 GitHub Release。

证据 staging 保留失败状态、执行参数、退出码/耗时、模型/模板/fixture 身份、版本与脱敏前后文件 hash。上游固定合成 stdout/stderr 正文不上传，但记录其 hash/字节数及源 fixture 关联。未知文件、模型、数据/令牌、原始服务日志与二进制不进入证据 artifact；拒绝项生成安全 staging failure，不能吞掉此前 CI 结论。它不是通用秘密清洗器，增加新报告必须审查允许列表。

Windows 10 x64 优先，Windows 11 后续。Server 2022 CI 绿色仍不等于 Windows 10 用户 i5-8400/16GB 或独立无开发工具、离线机器通过。用户已返回匹配该包identity的Win10 build19044 / i5-8400短验报告，并声明安装过大部分开发工具且测试联网；RAM/VC预装不能推断。2026-10-01 07:52–07:53 UTC用户明确无相应机器，批准把无开发工具、离线运行和长期稳定性列为后期验证并继续开发。因此T05按当前缩定范围（Release包完整性/依赖闭包、CI真实运行和用户Win10短验）收口已完成，T06开始；原A20和长期稳定性要求保留为后期验证项，报告中的unverified/skipped不改成pass。验收不更改 ExecutionPolicy、Defender、网络或系统权限，不要求用户安装开发工具。
