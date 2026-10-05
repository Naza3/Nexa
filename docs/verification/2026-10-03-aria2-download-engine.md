# 受控aria2下载引擎验证记录

日期：2026-10-03。状态：选型已采纳，工作树集成与本机最终回归/独立审查通过；最终Windows因辅助job未获runner而受阻，真实MS/HF、新包与用户复验未完成。最新已发送产品仍33f0e17。本页不把下载诊断、旧实现CI、原版aria2或交叉编译证据转授新引擎。

## 源码与范围

- 产品工作树基于主线`8c82203c1ff2f73981575733bd81a88c6cfd4f8a`；本轮整合使用`codex/nexa-aria2-integration`作为待验开发检查点，不触发最终Windows工作流；主开发分支在启动阻塞解决前保持不变。旧实现[WindowsCI37135712318](https://github.com/Naza3/Nexa/actions/runs/37135712318)已成功且52份报告独立核对通过，不覆盖本轮aria2实现
- 独立源码构建分支`codex/nexa-aria2-build`，提交`01db921354be64b67b38f1afdaa23e09dc79c4bd`，tree`a8724e0ce3f07353de6cbd25f57c84683e89e792`；[CI37138664930](https://github.com/Naza3/Nexa/actions/runs/37138664930)与产品最终CI分开
- 固定输入与3份本地补丁见[来源锁](../../third_party/aria2/source-lock.json)和[构建锁](../build-lock.md#aria2下载组件锁工作树集成最终windows待验)：网络策略、payload硬限、IOFile NUL下溢修正。IOFile修正不是“官方1.37.0已修”的声明
- 决策与首片契约见[ADR0018](../decisions/0018-generic-download-engine-candidate.md)，目录/来源/显式登记继续遵循[ADR0017](../decisions/0017-model-discovery-and-catalog-download.md)

用户4B MS下载0B立即报redirect_rejected，而浏览器/手动下载后扫描可用；具体旧被拒host仍未知，不能归因为目录权限。新引擎在真实源及用户复验完成前，不能称该故障已修复；手动扫描反馈不等于加载、聊天或性能通过。

## 工作树已实现的行为

来源适配器仅从可信catalog生成初始URL、精确size/SHA。bridge调用download-engine监督固定aria2进程，不再自建HTTP/Range；初始host匹配MS/HF，重定向与实际下载socket按HTTPS/443/保守公网策略约束。保留SChannel默认证书链/吊销验证，OS AIA/CRL/OCSP是已披露独立平台边界，不承诺所有OS网络都经过下载gate。

单任务、固定argv与清洁env、关闭RPC/config/netrc，禁止用户URL/CA/header/credentials/代理注入。Windows创建时原子设置Job、HANDLE_LIST和System32优先加载策略，失败关闭；该策略只调整DLL搜索优先级，不是禁止全部非系统DLL。stdout/stderr持续有界读取，CR/LF解析只输出受控数字，不保存URL/query或原始网络错误。

aria2任务内重试/Range恢复之外，只有真实OS exit8允许上层在确认停写后执行一次全量restart，attempt1→2；其它错误不自动全量重放，日志中的errorCode不能授予重试。两次尝试及最终校验共享两小时总deadline，进度不充当fsync或完整性证明。

model-store创建受保护的新UUID任务目录。子进程及输出线程真正结束后，父端独立核验精确size/SHA，经同一取消CAS决定hardlink no-clobber发布；已存在目标不覆盖，成功saved=true/registered=false。发布后清理未确认保持真实警告；回收未确认保留资源且fail-closed，不能因UI十秒关闭等待超时而abort后台future。跨App重启恢复与代理不属于首片产品支持。

## payload硬限与本地修复

父进程从可信expected_size生成`NEXA_PAYLOAD_MAX_BYTES`，不合并ambient同名变量。只接受canonical十进制4..17179869184；sidecar缺失/非法值在创建网络任务前拒绝，write/truncate/allocate/open/init先检查限额。该门槛覆盖单payload逻辑长度，不是累计下载流量、内存、metadata或整盘配额；受保护暂存与最终size/hash仍不可省略。

IOFile两处在strlen得到0时受控拒绝，避免len-1下溢；只处理所发现的问题，未宣称覆盖所有嵌入NUL语义。ASan/UBSan仅覆盖IOFile翻译单元和相关测试代码，不是全DiskWriter/libaria2或泄漏验收。

## 组件身份与残留规则

桌面包固定`download/nexa-aria2.exe`，同目录包含完整对应源码归档、原字节build-manifest、产品manifest/SHA256SUMS与11项固定许可原文。每次下载重新核对编译时source-lock、来源提交、完整文件/源码包/许可hash、实际AMD64 PE普通imports系统白名单并拒绝delay imports；Windows文件与祖先guard持有到bridge/reaper结束。hash证明字节一致性，不冒称发布方签名。裸runtime包不因此含sidecar；测试EXE/log/私有fixture不进入产品。

许可原文包括aria2/COPYING、MinGW/运行库/winpthreads/winstorecompat及LLVM compiler-rt/libc++/libc++abi/libunwind与工具包许可。源码归档包含完整修改后源与新增header、原始tar、原样3补丁、来源锁、构建/配置记录；具体对应源码与Nexa自身许可安排仍按发行材料核验，本页不作法律结论。

包内只在root、model、models直属位置惰性识别`.nexa-download-<canonical非nil UUID>/`；可为空，最多含以下3个普通文件：payload.part≤16GiB，payload.part.aria2与payload.part.aria2__temp各≤1MiB。新任务目录与旧UUID.part合计最多64项。只查元数据，不读取正文、不自动恢复/信任/执行/删除；未知文件、DLL、nested、symlink/reparse或其他位置拒绝。cleanup_warning若源于未知对象仍被拒，不能用“残留”豁免任意内容；普通扫描/模型准入未放宽。

## 验证结果分层

| 层 | 实际结果 | 不覆盖 |
| --- | --- | --- |
| 早期原版Linux aria2 | HTTP 7项PoC通过，含正确拒绝；独立size/hash与wrapper清理 | 网络策略补丁、Windows、产品发布 |
| 早期Linux网络原型 | 68 policy、26真实Request、31 integration/socket；独立重跑68+26并核对31 | Windows与真实MS/HF；不与原HTTP7项合并成产品通过 |
| 早期原版Windows | 4个fixture有独立通过记录 | 修改版网络/payload/IOFile补丁 |
| 辅助构建旧18bb | Windows68 policy/26 Request/4 SocketCore通过；随后错误要求WinTLS banner而失败 | 当时后续TLS步骤未执行；失败历史保留 |
| 辅助构建01db921 | 修正banner断言；Linux交叉构建通过，Linux53项真实DiskWriter/IOFile与12项HTTPS/硬限fixture通过 | AMD64 PE不是实际Windows运行；新3补丁Windows执行仍待验 |
| 产品监督器/model-store/bridge局部 | engine20、store11、bridge79曾通过，集合随后变化 | 不与聚合相加，不把旧bridge79视为新实现同等覆盖 |
| 主线工作树完整Rust | `cargo test --locked --offline --workspace --all-targets`退出0，40组389 pass/0 fail/7 ignored | 在最终crash-layout补充前运行；真实模型/包等ignored如实保留 |
| 工作树静态检查 | `cargo fmt --all -- --check`及完整workspace all-targets clippy `-D warnings`退出0 | Linux不证明Windows cfg分支 |
| UI | typecheck/lint、149 tests/9 files、build全退出0 | mock/控制器，不是目标机窗口或真实网络 |
| Python脚本聚合 | 126项，124 pass/2平台skip，退出0 | desktop11为其中子集，不重复累加 |
| 最终壳layout追加 | Linux28/0/0，all-targets clippy/fmt/diff通过；独立补审无新增阻断，desktop11亦独立重跑通过 | Windows junction测试已准备未执行；未据追加结果宣称重跑完整workspace |
| 独立审查 | 网络/payload/IOFile、组件身份/打包、最终残留规则各自已审 | 最终真实Windows产品链、原始ZIP字节闭包仍待新CI后复核 |

package_desktop的source_identity初次误用未定义stage/manifest已修复，验证移入实际verify并增加回归，未跳过门禁。早期网络fixture的DNS AI_NUMERICHOST与缓存重试/重解析证据修正保留于原型记录；后来的成功不抹去早期失败。

## 最新辅助CI：Windows尚未执行

01db921的Linux build及Linux fixture成功；Windows job111249100173在约2秒内失败，steps=[]、runner_id=0且runner_name为空，没有执行代码或产生Windows报告。此为尚未运行的job，不能记作断言失败或Windows通过。主代理于17:05 UTC成功请求仅重跑失败jobs，复用已完成Linux产物。第二次Windows job111249990877仍为零steps、runner_id=0，未执行代码，启动失败原因未知。

API没有提供可解释原因的annotations；云浏览器访问私有仓库返回未登录404，公开GitHub Status当天也未找到相关报告，均不足以确定根因。17:10 UTC已请用户提供run顶部错误原文或截图，等待该证据，不第三次盲重跑、不猜测计费原因。最终集成Windows验收暂受阻，主线最终代码尚未push；不因此修改源码或放宽验收门槛。

主线旧8c82203的CI37135712318已成功；52份报告大小/hash/提交身份与四项source fixture已核对，Rust49组368/0/7、CTest4/4、旧下载器MS固定0.6B共639446688字节/41888ms通过。此结果仅对应原下载实现，不能替代aria2整合检查点的最终CI。最终产品流程要求同source组件构建→Windows策略验证→真实MS固定0.6B下载→独立完整性/真实模型链→完整包与解压bridge；目前新引擎真实MS/HF、Windows进程/文件保护及包交付尚未执行。

## 本轮完成条件与后续

待精确集成提交的完整WindowsCI、真实源下载/取消/恢复/发布与独立包复核后再交付。旧具体redirect根因与用户新引擎复验分别记录；HF及目录中其他候选未实测时不扩大已验证模型矩阵。清洁机、离线、长期稳定性与更广设备仍按W05后期安排，不额外设为本片完成前置。Harness新产品工作继续暂停。
