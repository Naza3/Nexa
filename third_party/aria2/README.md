# Nexa 专用 aria2 Windows 构建候选

状态：独立构建/策略验证材料，**不是已集成产品包或发行验收**。网络补丁沿用
2026-10-03 隔离 Linux 原型的精确字节；原版 Windows 四 fixture 的通过不能
转授给这个修改版。生产集成、最终包字节身份和实际 Windows CI 结论由主线
验收分别记录。

## 固定来源和依赖

`source-lock.json` 是本构建的输入锁：aria2 1.37.0 官方 release tar.xz、
专用网络补丁、官方 llvm-mingw 20240619 UCRT/LLVM 18.1.8 Linux x64 工具包
以及四项 LLVM runtime 许可文件均按 SHA256 校验。
`patch` 保留原网络补丁；`additional_patches` 依次为 payload 硬限与 IOFile 本地修复，
三者分别校验，不用新补丁悄悄替换旧网络补丁。工具包 hash 来自本次
取得的官方 release 字节，不宣称已经独立验证上游签名。

- [aria2 官方 release](https://github.com/aria2/aria2/releases/tag/release-1.37.0)
- [llvm-mingw 官方 release](https://github.com/mstorsjo/llvm-mingw/releases/tag/20240619)
- [工具链构建材料](https://github.com/mstorsjo/llvm-mingw/tree/20240619)
- [固定 MinGW runtime 源码](https://github.com/mingw-w64/mingw-w64/tree/7c9cfe6708cafc83c14a2654308b2db62b126eae)
- [LLVM 18.1.8 源码](https://github.com/llvm/llvm-project/tree/llvmorg-18.1.8)

使用系统 Schannel/UCRT；LLVM C++/unwind/compiler runtime 和 MinGW 支持库
静态链接。不随包放置 OpenSSL、CA bundle、zlib、XML、SQLite、c-ares、SSH、
BT/Metalink 库。禁用 NLS 与 WebSocket。系统 DLL/API-set 导入清单先按明确
集合校验，仍须在 Windows 实际启动，不能用 PE header 代替 loader 验收。
`_WIN32_WINNT=0x0A00` 是编译基线，**不是所有 Windows 10 版本均已验证**。

构建 host 的 bash/make/patch/python3/pkg-config 来自 runner；工具链归档
固定，不宣称整个操作系统镜像或所有 host 工具可逐字节复现。没有源码全树
或工具链/EXE 入库。

## 构建和检查

Linux x64 host（Python 3.12、bash、make、patch、pkg-config）：

```sh
python3 -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_aria2_build*.py'
bash scripts/build_aria2_windows.sh /absolute/empty-work-directory
```

仅生成一份 out-of-tree Release/O2、无 debug 信息的构建；默认两个并行任务。
工具包只展开 x64 所需内容。下载及复用归档均校验 SHA，源树复用校验完整
文件快照，未知已有源/工具目录不复用。不是任意外部来源安装器。

`check-config` 必须确认 `SECURITY_WIN32`、`ENABLE_SSL`，逐一拒绝非预期
TLS/协议/依赖。补丁的五种编译 guard 分别执行“必须编译失败”的测试。
`policy_unit.exe` 使用生产 header，`engine_unit.exe` 链接真实 libaria2，
不复制/模拟 Request 或 SocketCore。输出仅为独立 artifact，不修改主 CI、
桌面包、runtime worker、引擎接口或桥。

Windows x64 host：

```powershell
python third_party/aria2/tests/windows_probe.py --artifacts <artifact-directory> --output aria2-windows-policy.json
```

测试先核对锁和三个 EXE 的 hash，检查：

1. 68 项地址/URI policy 单测
2. 26 项真实 Request 初始 URI/redirect 解析，4 项真实 SocketCore 私网目的拒绝
3. 53 项真实 DiskWriter/IOFile 单测（写入、mmap、truncate、allocate、open、溢出、NUL 行）
4. `--version` 的版本/禁用特征；后端用已校验 hash 的 config/PE 判断，
   不能要求上游未输出的 WinTLS banner，之后实际 TLS 测试独立证明运行行为
5. loopback 443 listener 观察下的 literal/integer/hex/octal/mapped 拒绝；listener不得收到连接
6. 初始 HTTP、非443、userinfo、fragment，以及 proxy/RPC/关证书校验/netrc/header 拒绝
7. 公网 HTTPS `example.com` 成功，badssl 错主机名/自签/过期证书拒绝

私有 env `NEXA_PAYLOAD_MAX_BYTES` 由父进程从已验证 expected_size 显式构造。
只接受 canonical ASCII 十进制，无正负号/空白/前导零，范围 4..17179869184。
transfer prepare 最前验证，缺失/非法值在创建网络任务前失败；`--version` 和
纯 policy/Request 单测不要求此值。probe 对下载场景显式提供限额，并在真实
Windows listener 下检查缺/坏限额导致零连接。

`DefaultDiskWriter` 捕获不可变限额；writeData 在 mmap 或系统写入之前、
truncate、allocate、open/init 都检查 offset/length。使用减法比较避免加法
溢出，不能在 cast 或 OS 操作后检查。payload 的未知长度/chunked、续传也走
同一路径；`.aria2` 进度元数据使用独立 BufferedFile/SHA1IOFile，不被 payload
上限误伤。父进程最终大小/hash 与受保护发布仍不可省略。

Linux job 额外执行 `tests/run_linux_payload.sh`：12 个实际 HTTPS 本地 fixture，
包括精确边界、chunked/Range 成功、超大长度/chunked/Range/预分配拒绝和
缺/坏限额的零网络拒绝。独立 LD_PRELOAD 仅映射测试地址、观察系统写入后的
文件大小，**不实现限额**；源补丁内没有测试旁路。测试 CA/密钥临时生成，
不进入 artifact；仅这个 Linux fixture 显式信任临时 CA。另对 IOFile 翻译
单元和53项测试启用 ASan/UBSan，不是全 aria2 sanitizer 或泄漏验收；LSan
在部分 sandbox 不支持，故明确禁用。Linux 结果不能转授 Windows。

证书负例必须出现明确的 Schannel 证书错误码，网络超时、吊销服务不可达、
通用握手失败均**不能充当通过**。不关闭证书或吊销检查、不导入测试 CA、
不改防火墙/hosts，不创建 RPC，不发放新权限。公共 fixture 不可用时 CI 应
失败并保留结果，不能改弱测试标准。

Windows 不使用 Linux LD_PRELOAD 网络重映射；因此本片没有 Windows
public-to-private redirect 完整 socket trace、DNS rebind/缓存失效证据、
真实模型/CDN、续传/取消、Job 清理或产品发布事务证据。Request 的 redirect
单测不冒充网络集成测试。OS DNS、Schannel 自动证书链/吊销的 AIA/CRL/OCSP
是独立平台边界，保留原行为，不承诺所有 OS 网络经过下载 socket gate。

## 分发材料和许可

`artifacts/` 包括固定命名 `nexa-aria2.exe`（未来产品路径 `download/nexa-aria2.exe`，此片不改打包器）、三个测试 EXE、PE 导入/编译器/配置/guard
记录、许可原文，以及 `build-manifest.json`（所有文件的大小和 SHA256）。
构建 manifest 的 `windows_runtime_tested=false` 保持不变；Windows job 的
独立 JSON 绑定它的 hash 和同一提交，才是实际运行结论。18bb 基线首轮 Windows 的68/26/4 native测试
已通过，但随后 version 测试错误要求 WinTLS banner 而失败，后续 TLS 等case
当时未执行；这次修正不会把未执行步骤追认为通过。

`aria2-1.37.0-nexa-corresponding-source.tar.gz` 包含：

- 完整干净的修改后 aria2 源树（显式含新增 `NexaNetworkPolicy.h`）
- 官方原 tar.xz、原样补丁、输入锁、构建脚本、测试、独立 workflow、说明
- 精确 `config.h`/`config.status` 与 aria2 `COPYING`/工具链 runtime 许可

这个 archive 是 aria2 对应源码材料，不是预先授予法律合规或发布许可的结论。
编译器完整源码不内嵌；标准工具链和静态支持库的固定来源在锁内列出，许可
原文随产物交付。任何发行需要再核实精确链接组成、Nexa 自身 LICENSE、GPL2+
对应源码可取得方式/保留义务，以及组合分发安排。14天 CI artifact 保留期
**不能替代正式发行的源码供给安排**。不要把三个测试 EXE、私有夹具、临时
文件或构建日志混入最终应用包。

## IOFile 本地修复与受限入口

原样 1.37.0 的 `IOFile::getLine` 和 `getsn` 先 `strlen` 再访问 `buf[len-1]`，
NUL 开头/缓冲分段开头可令 len=0。详见
[上游 issue2375](https://github.com/aria2/aria2/issues/2375) 与
[PR2376](https://github.com/aria2/aria2/pull/2376)。源码调用者还包括
UriListParser、Netrc、NsCookieParser、ServerStatMan，不仅是 `-i`。

新增 `nexa-iofile-nul.patch` 是本次依据实际源码编写的小修复：两处在 len=0
时抛出受控异常，避免 len-1 越界；未把外部 PR 盲搬，也不宣称上游已合并。
空文件、空行、正常行、NUL 首字节、跨4KiB分段NUL均有真实实现回归；仅修复
这里的 underflow，**不宣称处理了所有嵌入NUL的文本语义或所有上游漏洞**。
18bb 旧基线不含此修复。新的 Windows 执行与独立审查结论仍由精确 CI 记录。

专用调用方仍必须使用固定 argv、`--no-conf --no-netrc=true`，禁止任意
input-file、load-cookies、server-stat-if、配置/凭据/CA/用户参数注入。
补丁不是任意 aria2 CLI 的 sandbox，不能改为系统 aria2、用户自选二进制
或打开未审的新协议入口。

## 待推进边界

本轮新增逐次写入/预分配的 expected_size 硬上限，不改下载状态机。可信初始
目录、固定 argv/清洁环境、sidecar 身份、受保护文件事务和最终 size/SHA256
仍由父进程承担。最终大小/hash 或轮询 watchdog 不等价于写入前大小门槛。
Linux 上述 fixture 与 Windows native/公网策略测试分开报告；真实 MS/HF
大模型、产品事务/取消/Job、Win10目标设备和发行许可安排仍需后续验收。
