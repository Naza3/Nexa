# ADR0018：采用受控aria2伴随进程作为下载执行器

2026-10-09更新：启动库存仅校验声明产品文件，未声明内容不再扫描或拒绝；以下旧GGUF/下载残留例外规则由[ADR0041](0041-declared-payload-startup-validation.md)替代。模型准入和下载执行安全边界保持。

日期：2026-10-03。状态：选型已采纳，工作树集成进行中，尚未通过最终Windows/真实源/产品验收。主线基线为`8c82203c1ff2f73981575733bd81a88c6cfd4f8a`，已发送产品仍33f0e17；独立源码构建分支与产品集成分别验收。本文保留早期候选与隔离实验历史，最新集成范围见下节；原固定下载实现见[ADR0017](0017-model-discovery-and-catalog-download.md)，实际结果见[aria2验证记录](../verification/2026-10-03-aria2-download-engine.md)。

## 背景与当前取舍

用户要求通用下载引擎，并明确允许采用开源组件。目标仍是模型目录中的可靠下载，包含进度、取消、有限重试、正确续传与完整校验。用户手动下载4B后扫描可以，仅证明该操作链，不是加载/聊天/性能结论；原ModelScope重定向拒绝的具体目标仍未知，见[排查记录](../verification/2026-10-03-modelscope-redirect.md)。

已采纳aria2 1.37.0受控独立进程，随产品提供固定组件，用户无需另装。HTTP、重定向、重试与Range由aria2负责，不继续实现自建HTTP/Range引擎；此前reqwest草稿不是本轮通过证据。源适配器、传输监督器与父进程发布分别负责来源、子进程生命周期和文件事务，公开runtime协议与推理引擎不变。

## 首版职责与接口

| 层 | 工作树实现职责 | 尚须证明 |
| --- | --- | --- |
| 源适配器 | 从固定目录解析来源/revision、精确size/hash与受控请求信息；不把用户任意URL直接交给执行器 | URL/重定向策略与来源身份不被重试或续传改变 |
| aria2独立进程 | 单任务、固定私有暂存目录/文件；关闭RPC及无关协议/自动配置；传输、有限重试、任务内续传与有界进度 | Windows进程启动/关闭、资源与文件边界、网络策略、日志脱敏；不把退出0等同发布成功 |
| Nexa父进程 | 任务状态/取消与总deadline；确认子进程结束，独立核验最终size与SHA256，再执行安全no-clobber发布 | 目录/reparse/文件身份、写入者与替换竞争、发布原子性、终态及清理事实 |

下载成品仍不自动登记或加载。父进程必须保留真实发布事实，取消后不得留下可误用的“完成”状态；子进程退出未确认时不能提前放弃保护或宣称清理完成。子进程只接触本任务持有的暂存对象，不能根据下载URL或Content-Disposition自由决定最终路径。

原固定下载器采用同句柄写入/校验/发布；sidecar工作树已调整为任务目录与持续文件身份保护，外部写者停止后重新取得受保护文件用于独立校验/发布。Windows共享模式与TOCTOU边界须实际验证，不要求外部组件机械复用旧写入接口，也不能只因最终hash正确就放弃保护。

跨重启恢复、代理与更广的下载管理能力留后续；隔离PoC即使通过部分用例，也不等于Nexa已提供这些产品能力。

## 已实现的集成范围与未验边界

- `desktop-bridge::source_adapter`只把可信内建catalog转换成固定URL/size/SHA；初始host与所选MS/HF匹配，后续公开HTTPS跳转由sidecar策略约束，不再维护各CDN固定host名单
- `download-engine`监督单任务aria2，固定argv/env、关闭RPC/config/netrc，单流、不预分配、不覆盖；Windows创建时原子附加Job、允许继承的句柄及System32优先加载策略，不支持即失败关闭
- 上层只在OS退出码8时，确认writer真正结束后重置本任务暂存对象并允许一次全量restart；`attempt`从1到2，与aria2内部重试计数分开。两次尝试与最终验证共享同一两小时deadline，取消不会重置计时
- CR/LF进度解析与双输出管道有界，仅保留受控数字，原始URL/query/错误正文不进入UI或持久日志。进度不是fsync、完整性或最终成功证明；重启可显示新attempt与从零开始的进度
- model-store为sidecar创建本任务`.nexa-download-<UUID>/`暂存目录与固定`payload.part`；确认进程退出后重新取得受保护文件并独立核验精确size/SHA，再经同一取消CAS执行hardlink no-clobber发布。最终saved=true仍为registered=false，用户显式扫描/加载
- 壳与下载验证器仅使用经过组件manifest/source-lock/文件hash/许可/源码包/PE检查的`download/nexa-aria2.exe`；组件与目录保护持有到真实回收结束，不能使用系统或用户任意aria2替换

进程创建的System32优先策略只调整搜索次序，不等于禁止全部非系统DLL；不能替代包身份与文件/目录保护。无法确认进程/管道回收时继续保留资源且fail-closed；UI十秒关闭等待超时不能abort后台监督或假称已清理。

崩溃残留已增加严格惰性识别：仅root/model/models直属canonical非nil UUID任务目录，空目录或payload.part≤16GiB及两个固定control各≤1MiB；与旧UUID.part合计≤64，拒绝未知/nested/reparse。只查元数据，不自动恢复、信任或删除；最终Windows效果仍待验，详见本轮记录。首片不提供跨App重启恢复或代理支持，早期PoC的重启续传不能转授为这些产品能力。

## 首版网络边界

首版约束初始URL、每次重定向与aria2实际下载socket。仅接受可信内建catalog；父进程提供固定argv、清洁环境与私有暂存目标，不允许用户透传任意URL、CA、header、credentials、配置或CLI选项，RPC关闭。官方aria2 1.37.0原版配置未直接满足所需逐跳约束，因此已隔离验证小型专用策略补丁；该策略现由固定源码构建与工作树集成引用，最终Windows和产品组合仍待验。

约束须在请求/连接发生前拒绝非HTTPS降级与不允许的地址，覆盖重新解析、重连与跳转。当前原型在真实connect使用的同一sockaddr上检查443与保守公网分类，不另做一次可能与实际连接不同的DNS解析。下载前DNS预查不能代替此挂点，最终SHA256只能证明收到的文件内容，不能撤销先前请求，也不等价于防SSRF。代理会改变连接边界，本专用方案禁代理；地址分类也不保证NAT/VPN或系统路由后的物理目的地。

Windows构建保留SChannel默认自动证书链与吊销验证，不关闭TLS校验，不为满足过强的全网络承诺新造TLS或PKI。Windows OS为证书验证发出的AIA/CRL/OCSP检索属于独立平台边界，可能绕过aria2下载socket gate；本专用方案不承诺所有OS网络都受该gate约束。该边界已作为披露的残余风险接受用于继续集成验证，先前针对“所有下载触发联网均被gate覆盖”的阻断不再作为首版前提；Windows实际行为仍须验证。系统证书联网机制参考[Microsoft Crypt32说明](https://learn.microsoft.com/en-us/windows/win32/seccrypto/certificate-revocation-list-semantics)。

官方版本与源码入口：[aria2 1.37.0发布](https://github.com/aria2/aria2/releases/tag/release-1.37.0)、[版本固定手册](https://github.com/aria2/aria2/blob/release-1.37.0/doc/manual-src/en/aria2c.rst)。当前边界不授予任意用户URL/认证/代理支持，也不把成熟组件或Linux测试结果当Windows验收。

## 早期隔离PoC证据与边界

使用官方1.37.0源码包，SHA256为`60a420ad7085eb616cb6e2bdf0a7206d68ff3d37fb5a956dc44242eb2f79b66b`。本机Linux编译禁用BitTorrent/Metalink；运行时关闭RPC、读取配置与netrc，使用单连接、受控暂存文件和有限尝试。隔离PoC已完成并冻结，7/7判据通过，其中包含预期安全拒绝；所有请求仅在本地HTTP fixture，不能替代Windows/HTTPS或真实模型下载。源码归档hash为本机记录，未宣称独立核过发布方签名。

| 用例 | 已观察结果 | 结论边界 |
| --- | --- | --- |
| 完整8MiB | 退出0；外部Python独立size/SHA256相符 | 合成文件，不是GGUF/模型验收 |
| 2MiB后断流 | 任务内发Range、206恢复并通过完整hash | 仅此受控断流场景 |
| 服务器忽略Range回200 | 退出8，未被验证为完成 | 保留残留文件，不是自动清理成功 |
| 续传内容不一致 | 退出32，最终SHA不符而拒绝 | hash拒绝不能代替网络策略 |
| SIGTERM后重启 | 借助残留控制文件续传并通过hash | Linux进程重启PoC，不是Nexa跨重启恢复 |
| SIGKILL后重启 | 从保留状态恢复并通过hash | 未覆盖Windows Job、崩溃持久化或所有截断点 |
| 终止与清理 | wrapper确认终止后清理本次.part及.aria2 | aria2终止本身保留残留，清理由PoC wrapper执行 |

固定fixture为8,388,608 bytes、SHA256`78c6ad0a86e461c7de8eca55f8369eaa7b60aa00eeb7e730ecfdc12ad95b4bef`；本机Linux ELF SHA256为`d8bbf0604732cdbd5b9e1b2f157d49c9c3f3dbcf252acd381fd0cbb1e60c0fe9`。PoC报告SHA256为`1b9734b35bc90b29bdda818422c610d06b3639f8fdcae787904e408ed6d97bf7`，逐项命令/请求/结果JSON为`0239c4de37fac9bb279745bcd9314718f1d9d72f1d2f1640387880fe84c49a37`；19项证据文件hash已按封存清单核对。构建日志容器时钟与本会话日期不同，原记录未改。

当前Linux重定向console只有LF、没有CR，不能推断Windows进度格式；CR/LF增量、分片、超长行与背压仍待测。编译仍包含XML-RPC支持，只是每次运行明确关闭RPC；本轮不是裁剪后的Windows发行构建。外部Python仅做最终size/hash复核，未调用Nexa父进程安全发布、扫描或登记。这份未打策略补丁的7项PoC未验证HTTPS-only或逐跳地址约束；策略原型结果在下节单列。代理、Windows共享句柄/进程回收、真实MS/HF及生产包仍未验证，不能把7个PoC用例标为产品验收完成。

## 早期专用网络策略原型证据

隔离Linux原型补丁SHA256为`797bd6205909e0a762ac3973aa2b5b7df1ebbc50eb703123cd9eda0692bda966`；补丁覆盖6个原有源文件与1个新增头文件，沿用上游TLS/HTTP、调度、重试及续传。对原始官方源码重应用后内容逐字一致，补丁hash已核验。

| 范围 | 实际结果 | 边界 |
| --- | --- | --- |
| 纯策略 | 68项通过 | IPv4/IPv6、mapped地址、端口、scope及URI形状 |
| 真实Request | 26项通过 | 链接真实libaria2，检查解析与重定向 |
| integration/socket | 31项通过 | 含HTTPS跨host/相对跳转、私网/metadata/降级/凭据/代理等拒绝、不可信证书失败；使用隔离本地TLS fixture |
| 独立审查 | 独立重跑68+26并核对31项证据通过 | 不把核对31项写成全部独立重跑；不是Windows或公网验收 |

实际socket测试在生产地址检查之后，通过仅测试进程使用的LD_PRELOAD fixture把已许可地址映射到本地TLS服务；生产补丁没有loopback例外或测试环境开关。缓存重试与真实SocketCore重解析分开验证：重解析fixture先返回公网、再返回私网，轨迹确认第二次私网没有connect。当前是Linux/OpenSSL原型，未运行真实MS/HF或Windows SChannel。

原型禁止代理/RPC/用户凭据与header等输入，并在构建时拒绝BT、Metalink、SSH及异步DNS配置。固定argv/env仍是必要边界；不能把它解释成任意不可信CLI的沙箱。Windows方案继续采用上节明确的平台证书边界；SChannel自动验证不是已删除或绕开的安全检查。原HTTP 7项续传PoC与本HTTPS策略版没有完成组合验收，不能互相转授通过结论。

## 固定源码与本地补丁

当前输入锁见[`third_party/aria2/source-lock.json`](../../third_party/aria2/source-lock.json)：官方1.37.0源码、llvm-mingw 20240619/UCRT/LLVM18.1.8工具包及运行库许可均按hash固定。网络补丁保留原字节，另加payload写入前硬限及IOFile本地修复，两者不能冒称官方1.37.0已修复或上游已合并。

payload硬限来自可信expected_size，以canonical十进制通过清洁环境传给sidecar；缺失/非法值在创建网络任务前拒绝，write/truncate/allocate/open/init检查4字节至16GiB范围，避免先写后检查与加法溢出。它仅限制单payload逻辑长度，不是累计流量、内存、元数据或整盘配额，父端最终size/SHA仍不可省略。IOFile修复只处理所发现的`strlen==0`后`len-1`下溢，不宣称覆盖所有NUL语义或上游漏洞。

## 版本与分发准备

官方aria2 1.37.0为GPL-2.0-or-later，见[固定版本版权声明](https://github.com/aria2/aria2/blob/release-1.37.0/src/AbstractCommand.cc)与[COPYING](https://github.com/aria2/aria2/blob/release-1.37.0/COPYING)。Nexa仓库目前没有项目自身的根LICENSE文件；不能据“独立进程”自动得出整体许可结论。组件构建与打包已安排许可原文、完整修改后源码归档、原始源码/补丁/锁/构建记录和文件hash随组件提供；最终产品包与来源一致性仍须复核，并明确Nexa自身许可安排。CI artifact保留期不能替代发行源码供给。当前尚未交付aria2产品包，本ADR不作法律结论。

## 本轮交付门槛

1. 将历史PoC、辅助源码构建、当前产品工作树和目标设备证据分层，完成最终回归与独立审查
2. 在Windows验证三份补丁、受控进程/文件事务与真实源，保留SChannel自动证书/吊销检查；交叉PE不替代实际运行
3. 在Windows完成单任务、取消/强制回收、暂存身份、独立完整性与no-clobber发布闭环
4. 对真实MS/HF、断流/错误续传、目录竞争与日志隐私逐项验收，复核完整包后再交付；清洁机/离线/长期稳定性仍为后期条件，不新增为本片完成前置

5266ab6/8c82203的诊断CI、有界路由观察以及早期原版Windows fixture只证明各自版本，不覆盖本工作树的aria2引擎。具体旧被拒host仍未知，新真实源与用户复验前不能宣称问题已修复。
