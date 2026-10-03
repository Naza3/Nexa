# Windows 模型目录自动发现与双源下载验证记录

日期：2026-10-03。状态：进行中。用户2026-10-03 10:47 UTC确认优先完成模型加载，本片恢复目录发现、目录内下载→扫描/加载；后端检查点和前端单独验证已提供，本机完整聚合及后端/UI审查已通过，主代理补充CI/许可/证据门禁后已冻结源码并通过本机最终检查，Windows真实产品链仍待验。用户要求优先解决models目录未自动识别，并增加默认ModelScope、可选Hugging Face的目录内下载。

## 基线与证据范围

- 开始时HEAD为`4d30bfae815dbdce888f58ec6bf911834dc8dca9`，tree`ba0a30d4f56dfcb86d8fc85cf1cb86ccab2cf1c5`；已发送产品仍43ad5c2，用户下载/运行未确认
- 4d30bfa是T0无模型parser证据实验，其CI37115797798与当前下载实现分开记录；已success并独立核50报告，仍不覆盖当前工作区增量
- [ADR0017](../decisions/0017-model-discovery-and-catalog-download.md)规定发现优先级、显式双源下载、保存与登记分离及旧设置回退；本片不扩工具能力、不建立模型名单、不修改旧权重文件
- 内置8条目录来自固定revision的双服务公开元信息研究：8条MS端点HEAD 200及精确Content-Length，HF端点观察为302到HF CDN，研究未跟随最终CDN响应或下载权重。两源声明size/hash相等不是本机完整hash复算
- 仅Qwen3-0.6B Q8_0原固定文件有既有Windows真实推理证据；其他7个文件仍未实测候选，目录/下载成功不会授予validated

## D01–D14 分层验收矩阵

| ID | 门槛 | 当前证据 |
| --- | --- | --- |
| D01 | 未配置且停止时发现EXE/models；已配置/失效/stale优先，无CWD/model回退 | Linux bridge/前端合成通过，Windows实际EXE位置/窗口待验 |
| D02 | 无目录/全坏/混合/空目录及原有partial事务真实状态 | 既有事务回归和新发现合成通过，目标目录真机待验 |
| D03 | 固定8条、双源revision/大小/hash、合法本地模型不受名单限制 | 元信息、内置JSON与研究一致；解析/身份反例通过，不授予其余7条模型能力 |
| D04 | 启动/列表无网络；显式下载才联网，源与目录在开始时绑定 | 后端/UI调用与源码审查通过；未跑真实下载 |
| D05 | 默认MS/保存HF、旧设置缺字段兼容、新设置回退旧版步骤 | 偏好持久化/缺字段/非法源回归通过；旧版严格unknown-field边界已核，真实降级窗口未跑 |
| D06 | 服务运行快拒，不自动停服务；重复下载/扫描等并发互斥 | admission、实例锁、active任务快拒及snapshot/close合成回归通过 |
| D07 | 受控HTTPS/host/redirect，无凭据/代理/隐式源fallback | 精确URL策略/反例及UI无fallback通过；真实TLS/重定向网络仍待验 |
| D08 | 真实字节/精确size/hash、短流/超长/错误编码/HTTP失败不发布 | 本地有界HTTP响应fixture通过；Windows完整写者hash/发布用例与真实MS传输待验 |
| D09 | UUID.part、目录身份/reparse、no-clobber与途中目标竞争 | Linux文件事务/竞争测试通过；Windows句柄/祖先/hardlink/disposition未执行 |
| D10 | 取消/超时/关闭终态与清理；发布成功后警告不谎称回滚 | 控制竞争/关闭/结果解析单元与UI通过；真实Windows取消及清理故障未实测 |
| D11 | completed saved=true registered=false；手动扫描后才可加载 | DTO/UI成功反例及显式扫描关联通过；真实下载→扫描/加载链待验 |
| D12 | 包根/model/models精确.part例外，拒绝任意.part/链接/未知可执行文件 | 壳Linux24项/脚本逻辑回归通过；实际新Windows包与许可闭包待验 |
| D13 | Windows下载器经MS取得固定0.6B原字节，再用于既有真实链 | 已规划，尚未执行 |
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

后端/UI独立审查通过。主代理随后将Windows流程改为先经产品下载器从默认MS取得固定0.6B，再保留原独立基线hash及真实模型链；新增封闭JSON证据门禁、固定目录source fixture，并显式收集AWS-LC的aws-lc/LICENSE与嵌套fiat LICENSE。上层aggregate LICENSE已有相关内容（末尾空格有差异），本轮仍保留嵌套原文以完成可审计闭包；最终Windows产物许可尚待实际构建核验。初次desktop ACL检查因5个新命令失败，已更新精确预期集合和catalog_id-only请求断言后通过，没有跳过门禁。

后端新增opt-in下载验证器，可在Windows明确调用后经MS获取固定0.6B并独立复算hash；本机没有执行权重下载。许可文件收集测试不等于最终Windows产物许可/PE闭包已审；Windows guard/hardlink/disposition、实际MS传输、完整包与目标设备仍待验证。本片首次精确提交WindowsCI因桌面构建步骤超时失败，详情见下节；尚无独立新包复核或发送事实。主代理补充6文件审查范围SHA256为`210b4a72b083eec757e64d4246f0ad02367bcf62b50a72b5629e117b2dbb4b6d`，独立审查与最终格式/86项脚本/前端检查通过；该范围hash不等于最终Git或产品身份。后端18文件检查点、root6增量及最终前端范围分别保留，不能把较早检查点hash当作最终全树hash。

## 首次精确提交 Windows CI：桌面构建超时

实现提交为`817ad7d6174ea16ff6210438a8ea641562a9e84a`，tree为`31b6d681271f2bf4d814357b0307a81c0ab5ce03`。[Windows run37118858126 / job111190859892](https://github.com/Naza3/Nexa/actions/runs/37118858126/job/111190859892)失败于“Check independent desktop Rust graph + Tauri Release”步骤：2026-10-03 11:17:16–11:37:21 UTC达到20分钟步骤时限。主代理与独立审查核查日志：Windows壳23项测试于11:24:45通过，clippy于11:26:32通过；Release于11:26:33才开始，在共享步骤预算中实际得到10分48秒。最后在11:34 UTC仍为desktop-bridge/nexa-desktop正常编译，超时前未见编译错误。

模型下载、native、全workspace与包步骤均未执行，本次不能证明模型运行成功或失败。重试准备仅将该步骤时限从20改为35分钟，全局90分钟和全部检查保持，独立审查通过；重试提交与结果另记。历史Android研究[run37118858122](https://github.com/Naza3/Nexa/actions/runs/37118858122)由根Cargo.lock变化另行触发，现已成功，仅为native研究回归，没有APK或设备验收，与Windows产品验收无关。最新已发送产品仍为43ad5c2。

## 第二次 Windows CI：前端 preview 测试超时

重试提交为`4fa622cc0b97ebabb0251c774d8506a826577a85`，tree为`2fbb9909d3ff9752b48b8bf7dcd061755cb87102`。[Windows run37120635638 / job111195882779](https://github.com/Naza3/Nexa/actions/runs/37120635638/job/111195882779)在前端`preview.test.tsx`单项达到15秒时限后失败，其余104项通过。817ad7d同一preview测试此前在6878ms通过；目前没有证据证明生产竞态。

本次未到达独立桌面Rust图/Tauri Release步骤，不能据此判断新35分钟预算是否足够；模型下载、native、全workspace与包步骤仍未执行。下一次修正限定于preview测试的虚拟时钟：保留完整流程和全局15秒时限，以50ms步进、每阶段3秒虚拟预算明确等待ready与操作完成，并在finally恢复真实时钟。修正后的单例五次重复分别为1878/1973/1809/2024/1777ms，均通过；typecheck、lint、全105项、build及diff检查全退出0，独立审查通过。精确测试文件SHA256为`767103c32b3129974800d91aa07f40c5da93a847e509d6f31f866f5b313dcd02`；嵌套finally覆盖setup/render和unmount失败，恢复原URL、真实时钟与测试wrapper。生产功能源码未因此修改。原Windows超时未在本机复现，目前只能确认已移除该测试的墙钟等待与隐含就绪依赖，修正后的WindowsCI仍待验。

## 设置回退与当前限制

新增download_source在严格单文件desktop-settings内原子保存，旧文件缺该字段默认MS。旧43ad5c2读取新版设置会拒绝未知key，回退应恢复升级前备份或仅移除download_source并保留其他值；新版“恢复默认”仍写该key，不解决回退。

服务须先显式停止；下载成功只保存到已选目录，不自动登记/加载。其他本地合法GGUF仍可尝试，16GB是总内存、16GiB是单文件预算，两者都不是任意模型可运行承诺。最新可交付与具体来源结果以各自精确证据为准。
