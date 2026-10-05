# Windows 模型目录自动发现与双源下载验证记录

日期：2026-10-03。状态：33f0e17精确WindowsCI与独立下载包复核通过，原字节包已于13:42:49 UTC发送获接受，用户下载/运行未确认。用户2026-10-03 10:47 UTC确认优先完成模型加载；目录发现、默认ModelScope/可选Hugging Face下载→显式扫描/加载已实现。默认MS固定0.6B的实际下载和后续真实推理/完整包链路已验，原生窗口、HF实际下载、其余7个候选和目标机仍待验。前三次失败与修正完整保留。

## 基线与证据范围

- 开始时HEAD为`4d30bfae815dbdce888f58ec6bf911834dc8dca9`，tree`ba0a30d4f56dfcb86d8fc85cf1cb86ccab2cf1c5`；当时已发送产品为43ad5c2，用户下载/运行未确认
- 4d30bfa是T0无模型parser证据实验，其CI37115797798与当前下载实现分开记录；已success并独立核50报告，仍不覆盖后续目录下载增量
- [ADR0017](../decisions/0017-model-discovery-and-catalog-download.md)规定发现优先级、显式双源下载、保存与登记分离及旧设置回退；本片不扩工具能力、不建立模型名单、不修改旧权重文件
- 内置8条目录来自固定revision的双服务公开元信息研究：8条MS端点HEAD 200及精确Content-Length，HF端点观察为302到HF CDN，研究未跟随最终CDN响应或下载权重。两源声明size/hash相等不是本机完整hash复算
- 仅Qwen3-0.6B Q8_0原固定文件有既有Windows真实推理证据；其他7个文件仍未实测候选，目录/下载成功不会授予validated

## D01–D14 分层验收矩阵

| ID | 门槛 | 当前证据 |
| --- | --- | --- |
| D01 | 未配置且停止时发现EXE/models；已配置/失效/stale优先，无CWD/model回退 | Windows发现/目录前置12项通过；真实EXE窗口启动与目标机待验 |
| D02 | 无目录/全坏/混合/空目录及原有partial事务真实状态 | Windows目录12项及external17项通过；含混合/全坏/空目录事务，目标机窗口另验 |
| D03 | 固定8条、双源revision/大小/hash、合法本地模型不受名单限制 | 元信息、内置JSON与研究一致；解析/身份反例通过，不授予其余7条模型能力 |
| D04 | 启动/列表无网络；显式下载才联网，源与目录在开始时绑定 | 后端/UI与源码审查通过；Windows显式MS下载已执行，未启用启动/列表远程查询 |
| D05 | 默认MS/保存HF、旧设置缺字段兼容、新设置回退旧版步骤 | 偏好持久化/缺字段/非法源回归通过；旧版严格unknown-field边界已核，真实降级窗口未跑 |
| D06 | 服务运行快拒，不自动停服务；重复下载/扫描等并发互斥 | admission、实例锁、active任务快拒及snapshot/close合成回归通过 |
| D07 | 受控HTTPS/host/redirect，无凭据/代理/隐式源fallback | 精确URL策略/反例通过，MS真实HTTPS传输通过；HF实际TLS/重定向仍待验 |
| D08 | 真实字节/精确size/hash、短流/超长/错误编码/HTTP失败不发布 | 本地HTTP fixture、Windows受控写者size/hash/取消/发布与MS固定文件完整size/hash通过；不推广为所有网络故障已验 |
| D09 | UUID.part、目录身份/reparse、no-clobber与途中目标竞争 | Windows受控文件事务、目录身份/guard、no-clobber与实际MS发布通过；目标机及任意文件系统组合未验 |
| D10 | 取消/超时/关闭终态与清理；发布成功后警告不谎称回滚 | Windows控制竞争/有限关闭/发布结果与受控取消用例通过；真实MS下载途中用户取消及清理故障注入未验 |
| D11 | completed saved=true registered=false；手动扫描后才可加载 | 实际MS下载报告published=true/registered=false；同一固定文件随后独立校验并用于真实模型/包/bridge，UI手动扫描/加载窗口待验 |
| D12 | 包根/model/models精确.part例外，拒绝任意.part/链接/未知可执行文件 | Windows实际Release包、严格preflight/反例与解压bridge通过；下载原包独立许可/PE闭包复核通过，原生窗口待验 |
| D13 | Windows下载器经MS取得固定0.6B原字节，再用于既有真实链 | 33f0e17 WindowsCI通过：639,446,688字节、47,149ms、完整SHA256符合原固定基线 |
| D14 | 用户Win10/i5-8400/16GB原生发现、下载、取消、扫描、加载/性能 | 未执行；其他7个候选/实际HF下载/长期条件另验 |

## 实际实现与验证

后端18文件检查点集合hash为`2f1a2887a84a3714f9e2642e06b6f8b04c0addf2d617ddad7b99c67d38c6dc13`，为提交前的局部检查点。该检查点先有下列定向结果；其后完整聚合与主代理补充检查单列，不把计数相加：

| 范围 | 检查点结果 | 边界 |
| --- | --- | --- |
| model-store / desktop-bridge定向测试 | 123 pass/0 fail/0 ignored | probe bin加入前的Linux合成回归 |
| 下载验证器参数测试 | 1 pass | 不触发权重下载，不与真实下载等同 |
| 上述后端all-targets locked clippy，`-D warnings` | 退出0 | 最终后端检查点，完整workspace另验 |
| Tauri壳Linux测试 | 24 pass/0 fail | 不编译/执行Windows命令分支或文件保护 |
| 桌面UI独立测试 | 105项通过 | 组件/mock，不是真实网络或原生窗口 |

后续本机完整检查已报告通过：

| 范围 | 结果 | 边界 |
| --- | --- | --- |
| `cargo test --locked --workspace` | 退出0；49组360 pass/0 fail/7 ignored | 最终后端阶段完整聚合，包含前述定向检查，不累加；真实模型/官方pi-ai/真实包相关ignored仍如实保留 |
| 全workspace及独立Tauri壳all-targets locked clippy，`-D warnings` | 全退出0 | Linux静态检查，Windows命令分支仍待CI |
| 壳Linux测试 / UI独立测试 | 24 / 105项（9文件）通过 | 不冒称Windows原生窗口 |
| UI `npm run typecheck`、`npm run lint`、`npm run build` | 全退出0 | 主代理最终前端检查，未运行Windows GUI |
| `cargo fmt --all -- --check` | 退出0 | 最终格式检查 |
| `python3 -m unittest discover -s scripts` | 86项，84 pass/2平台skip，退出0 | 主代理完整脚本聚合；下列22/7/22为其子集，不累加 |
| `python3 scripts/test_windows_package.py` | 22项，20 pass/2 skipped | 打包逻辑/新增许可收集回归 |
| `python3 scripts/test_desktop_package.py` | 7 pass | 壳命令/ACL与打包契约逻辑 |
| `python3 scripts/test_stage_ci_evidence.py` | 22 pass | 封闭证据schema/身份门禁，不是真实远程下载 |

后端/UI独立审查通过。主代理随后将Windows流程改为先经产品下载器从默认MS取得固定0.6B，再保留原独立基线hash及真实模型链；新增封闭JSON证据门禁、固定目录source fixture，并显式收集AWS-LC的aws-lc/LICENSE与嵌套fiat LICENSE。上层aggregate LICENSE已有相关内容（末尾空格有差异），本轮仍保留嵌套原文以完成可审计闭包；该本机阶段尚未核验实际Windows产物许可，最终产物复核另记。初次desktop ACL检查因5个新命令失败，已更新精确预期集合和catalog_id-only请求断言后通过，没有跳过门禁。

后端新增opt-in下载验证器，可在Windows明确调用后经MS获取固定0.6B并独立复算hash；本机没有执行权重下载。许可文件收集测试不等于最终Windows产物许可/PE闭包已审；初始本机检查未执行Windows guard/hardlink/disposition；其后Windows分支的有限测试结果按各次CI单列，最终33f0e17已通过实际MS传输及完整包CI，目标设备仍待验证。各次精确提交WindowsCI失败、修正和最终成功详见下节；最终独立新包复核与发送事实见末节。主代理补充6文件审查范围SHA256为`210b4a72b083eec757e64d4246f0ad02367bcf62b50a72b5629e117b2dbb4b6d`，独立审查与最终格式/86项脚本/前端检查通过；该范围hash不等于最终Git或产品身份。后端18文件检查点、root6增量及最终前端范围分别保留，不能把较早检查点hash当作最终全树hash。

## 首次精确提交 Windows CI：桌面构建超时

实现提交为`817ad7d6174ea16ff6210438a8ea641562a9e84a`，tree为`31b6d681271f2bf4d814357b0307a81c0ab5ce03`。[Windows run37118858126 / job111190859892](https://github.com/Naza3/Nexa/actions/runs/37118858126/job/111190859892)失败于“Check independent desktop Rust graph + Tauri Release”步骤：2026-10-03 11:17:16–11:37:21 UTC达到20分钟步骤时限。主代理与独立审查核查日志：Windows壳23项测试于11:24:45通过，clippy于11:26:32通过；Release于11:26:33才开始，在共享步骤预算中实际得到10分48秒。最后在11:34 UTC仍为desktop-bridge/nexa-desktop正常编译，超时前未见编译错误。

模型下载、native、全workspace与包步骤均未执行，本次不能证明模型运行成功或失败。重试准备仅将该步骤时限从20改为35分钟，全局90分钟和全部检查保持，独立审查通过；重试提交与结果另记。该次失败时最新已发送产品仍为43ad5c2。

## 第二次 Windows CI：前端 preview 测试超时

重试提交为`4fa622cc0b97ebabb0251c774d8506a826577a85`，tree为`2fbb9909d3ff9752b48b8bf7dcd061755cb87102`。[Windows run37120635638 / job111195882779](https://github.com/Naza3/Nexa/actions/runs/37120635638/job/111195882779)在前端`preview.test.tsx`单项达到15秒时限后失败，其余104项通过。817ad7d同一preview测试此前在6878ms通过；目前没有证据证明生产竞态。

本次未到达独立桌面Rust图/Tauri Release步骤，不能据此判断新35分钟预算是否足够；模型下载、native、全workspace与包步骤仍未执行。下一次修正限定于preview测试的虚拟时钟：保留完整流程和全局15秒时限，以50ms步进、每阶段3秒虚拟预算明确等待ready与操作完成，并在finally恢复真实时钟。修正后的单例五次重复分别为1878/1973/1809/2024/1777ms，均通过；typecheck、lint、全105项、build及diff检查全退出0，独立审查通过。精确测试文件SHA256为`767103c32b3129974800d91aa07f40c5da93a847e509d6f31f866f5b313dcd02`；嵌套finally覆盖setup/render和unmount失败，恢复原URL、真实时钟与测试wrapper。生产功能源码未因此修改。原Windows超时未在本机复现，目前只能确认已移除该测试的墙钟等待与隐含就绪依赖，该修正随后在第三次WindowsCI通过前端105项；其余产品链路结果见下节。

## 第三次 Windows CI：目录测试路径显示断言

提交为`21cfb408d7a3254f0afa91aeb7b874e38cb7bd78`，tree为`8541a044d1aa7bbbe0281be47ee922e24c8832e9`。[Windows run37121623438 / job111198689967](https://github.com/Naza3/Nexa/actions/runs/37121623438/job/111198689967)已通过前端105项、Tauri Release和native构建/CTest4/4。Release耗时11分11秒，于2026-10-03 12:29:52 UTC完成，本轮35分钟共享步骤预算足够。

主代理核查job日志：Windows desktop-bridge单元测试18项通过，包含`windows_fixture_bytes_verify_size_hash_cancel_and_publish_without_registration`，只证明受控fixture的size/hash、取消、发布且不登记分支，不是真实MS下载。随后model_directory集成测试11通过/1失败；启动发现用例第410行直接比较`C:\Users\RUNNER~1...`和`\\?\C:\Users\runneradmin...`显示字符串，两者指向同一路径对象。真实MS下载、全workspace完成、native真实推理及完整包步骤均未执行。

修正仅在该测试断言两侧调用canonicalize，生产路径/身份保护未改。精确测试文件SHA256为`8d270aa012c5ad1179a24ba0611117be5fd269491db69e84f15e6fa3e5564562`；本机desktop-bridge74项（含目录12项）、clippy与格式检查通过。流程增加同一bridge的`--lib --test model_directory`前置检查，后续全workspace检查保留；证据stager增加该精确日志并通过22项测试。三文件修正独立审查通过，范围SHA256为`013c7d90661c5c7286511336ce07e6306345dff348896de4807184db6268c218`；前置检查与后续完整门禁均保留，实际目录断言仍有效。修正后的Windows结果另记。

## 最终33f0e17 Windows CI与产物

源码提交为`33f0e17a5bdaf5e5d234034af0c946d878e0e4ae`，tree为`e2aa33f7852c09514361fe94178156bfaa95d481`。[Windows run37124146573 / job111205956541](https://github.com/Naza3/Nexa/actions/runs/37124146573/job/111205956541)成功。证据索引结果pass、无拒收报告，52份白名单报告的精确提交、大小与SHA256已核验；前三次失败记录保留，最终成功不改写其未执行范围。

| 范围 | 最终Windows结果与边界 |
| --- | --- |
| 常规Rust workspace | 49组363 pass/0 fail/7 ignored；ignored单列，不与显式真实链计数累加 |
| 发现/下载前置检查 | bridge单元18项、model_directory12项通过，属于后续聚合覆盖子集；前次路径显示断言修正通过 |
| Native Release | 构建及CTest4/4通过，包括流缓冲、模板、模板隐私与无模型工具parser实验；不因此授予工具能力 |
| 默认MS实际下载 | catalog_id=`qwen3-0.6b-q8-0`；revision=`6abe20cd0aed577f4d0b267935868ecae190aee9`；下载及文件大小均639,446,688 bytes；47,149ms；success=true、published=true、registered=false |
| 独立完整性 | SHA256=`9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`，下载器独立复算后仍走原固定基线检查；实际GGUF模板hash=`57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361`符合原基线 |
| 后续真实链 | 同一固定GGUF的native/停止/core/worker/HTTP/CLI、独立Release包及解压desktop bridge通过；context2048、2线程、batch128；不授予其他量化/模型/参数证据 |
| 目录保护 | Windows external17项、进程隔离guard2项通过；既有mapping观察仍仅按实际字段解释，不扩张为所有可写mapping组合覆盖 |
| 桌面产物 | ZIP11,119,286 bytes；SHA256=`b37d89cbd1baf1dfd07d7dbdeae794157504d4c5a4064e171e7c4851d1015c1b`；UI EXE14,723,584 bytes，安装内容33,421,543 bytes，包内模型0 bytes |
| 包内验收 | acceptance与bridge_result为pass，package_unchanged=true；native_window_tested=false，Windows10真实窗口交互与剪贴板/关闭仍单独验收 |

47,149ms仅为该次CI网络下载观测，不承诺用户网络速度。运行环境仍为Windows Server2022 CI，不能改写为Win10/i5-8400/16GB实测。HF实际下载、其余7个目录候选及广泛模型性能未执行；产品下载器保存后不自动登记，真实窗口的显式扫描/加载流程仍待用户验收。下载原ZIP的独立PE/许可/字节闭包复核通过，原字节产物已发送；详见下述交付记录。

### 独立原包复核与交付

独立复核确认原ZIP含817个文件、嵌套runtime209个文件；实际6个AMD64 PE的普通与delay-load导入闭包完整，新增3个OS依赖均已分类，没有缺失DLL。许可索引595+6+198项及AWS-LC根LICENSE、native LICENSE与嵌套fiat LICENSE均保留；52份报告与4份固定source fixture核验通过。3个CRT文件与CI微软签名记录相符；Linux复核没有重新执行Authenticode签名验证，不能混称独立重验签。

2026-10-03 13:42:49 UTC，以`Nexa-Windows-x64-33f0e17.zip`发送11,119,286字节的原ZIP，SHA256为`b37d89cbd1baf1dfd07d7dbdeae794157504d4c5a4064e171e7c4851d1015c1b`；内容未修改，消息发送获接受，用户下载/运行未确认。已告知停止旧服务、解压新目录、默认MS下载后显式扫描再启动/加载，以及旧设置回退规则。HF实际下载、其余7个模型和Win10/i5-8400/16GB原生窗口仍待验；Harness新实施按用户要求暂停，生产工具调用未实现。

## 设置回退与当前限制

新增download_source在严格单文件desktop-settings内原子保存，旧文件缺该字段默认MS。旧43ad5c2读取新版设置会拒绝未知key，回退应恢复升级前备份或仅移除download_source并保留其他值；新版“恢复默认”仍写该key，不解决回退。

服务须先显式停止；下载成功只保存到已选目录，不自动登记/加载。其他本地合法GGUF仍可尝试，16GB是总内存、16GiB是单文件预算，两者都不是任意模型可运行承诺。最新可交付与具体来源结果以各自精确证据为准。
