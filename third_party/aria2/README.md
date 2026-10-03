# Nexa 专用 aria2 Windows 构建候选

状态：独立构建/策略验证材料，**不是已集成产品包或发行验收**。网络补丁沿用
2026-10-03 隔离 Linux 原型的精确字节；原版 Windows 四 fixture 的通过不能
转授给这个修改版。生产集成、最终包字节身份和实际 Windows CI 结论由主线
验收分别记录。

## 固定来源和依赖

`source-lock.json` 是本构建的输入锁：aria2 1.37.0 官方 release tar.xz、
专用网络补丁、官方 llvm-mingw 20240619 UCRT/LLVM 18.1.8 Linux x64 工具包
以及四项 LLVM runtime 许可文件均按 SHA256 校验。工具包 hash 来自本次
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
3. `--version` 的版本/TLS/禁用特征
4. loopback 443 listener 观察下的 literal/integer/hex/octal/mapped 拒绝；listener不得收到连接
5. 初始 HTTP、非443、userinfo、fragment，以及 proxy/RPC/关证书校验/netrc/header 拒绝
6. 公网 HTTPS `example.com` 成功，badssl 错主机名/自签/过期证书拒绝

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

`artifacts/` 包括固定命名 `nexa-aria2.exe`（未来产品路径 `download/nexa-aria2.exe`，此片不改打包器）、两个测试 EXE、PE 导入/编译器/配置/guard
记录、许可原文，以及 `build-manifest.json`（所有文件的大小和 SHA256）。
构建 manifest 的 `windows_runtime_tested=false` 保持不变；Windows job 的
独立 JSON 绑定它的 hash 和同一提交，才是实际运行结论。

`aria2-1.37.0-nexa-corresponding-source.tar.gz` 包含：

- 完整干净的修改后 aria2 源树（显式含新增 `NexaNetworkPolicy.h`）
- 官方原 tar.xz、原样补丁、输入锁、构建脚本、测试、独立 workflow、说明
- 精确 `config.h`/`config.status` 与 aria2 `COPYING`/工具链 runtime 许可

这个 archive 是 aria2 对应源码材料，不是预先授予法律合规或发布许可的结论。
编译器完整源码不内嵌；标准工具链和静态支持库的固定来源在锁内列出，许可
原文随产物交付。任何发行需要再核实精确链接组成、Nexa 自身 LICENSE、GPL2+
对应源码可取得方式/保留义务，以及组合分发安排。14天 CI artifact 保留期
**不能替代正式发行的源码供给安排**。不要把两个测试 EXE、私有夹具、临时
文件或构建日志混入最终应用包。

## 已知上游输入文件缺陷与受限入口

原样 1.37.0 的 `IOFile::getLine` 先 `strlen` 再访问 `buf[len-1]`，NUL 开头行
可令 len=0。网络补丁**没有修复该缺陷**；不要把版本固定写成没有已知问题。
详见 [上游 issue2375](https://github.com/aria2/aria2/issues/2375) 与
[PR2376](https://github.com/aria2/aria2/pull/2376)。本次源码复核发现调用者包括
UriListParser、Netrc、NsCookieParser、ServerStatMan，不仅是 `-i`。

专用调用方必须使用固定 argv、`--no-conf --no-netrc=true`，禁止任意 input-file、
load-cookies、server-stat-if、配置/凭据/CA/用户参数注入。网络补丁限制部分
选项，但不是任意 aria2 CLI 的 sandbox；尤其不能把系统装的 aria2 当候选。
这个受限入口降低暴露面，**不是缺陷已修复的声明**。未来 backport 必须独立
审查实际修改及 NUL-line 回归，不能因外部 PR 自动把未审补丁纳入生产。

## 待推进边界

本补丁只约束 initial/redirect URI 和 aria2 最终下载 sockaddr。可信初始目录、
固定 argv/清洁环境、sidecar 身份、受保护文件事务和最终 size/SHA256 仍由
父进程承担。尚未新增逐次写入/预分配的 expected_size 硬上限；应独立评估
`AbstractDiskWriter::writeData/truncate/allocate` 的统一溢出安全限制及可信
limit 注入，并测试超大 Content-Length、chunked、range 和预分配路径。
最终大小/hash 或轮询 watchdog 不等价于写入前大小门槛。
