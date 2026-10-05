# 2026-10-05 桌面源码与文档清理验证

## 范围与前置

任务：按用户要求，在便携ZIP/MSI/Setup安装器交付后移除本项目移动代码与相关文档，后续聚焦桌面。基线为长期`codex/dev`的干净提交`de7732f031c11e44a27f86b33a341c48131a3906`。前置交付来源和全部13项Windows安装生命周期通过事实见[最终三格式记录](2026-10-05-tag-release.md#最终de7732f原生成功与三格式交付)。本记录先保留其后的本机开发检查，再记录精确清理提交`f577a49861298ec278293ba2d7231e09c3c07022`的原生结果；不借用旧包结果证明清理后源码。

范围决策见[ADR0029](../decisions/0029-desktop-only-source-tree.md)。没有修改独立`Naza3/MNN`仓库或重写Git历史；没有操作tag、Release或main。

## 实际变更

- 删除166个移动工程专用tracked文件：验证器、独立移动workspace、MNN适配/探针/补丁、专用脚本、专属许可材料和两份Android CI
- 删除25个专用文档：旧跨端快照11份、移动计划/契约/验收5份、ADR0008–0013六份、移动验证报告3份
- 共享代码移除`RuntimeConfig::android()`与开发验收报告的`android_arm64_cpu`占位；零/一等待槽调度回归保持显式fixture，模型平台拒绝回归改用未支持的Windows arm64值，通用流式导入保留
- Windows工作流移除移动路径忽略项；新增5项项目源码边界检查，保持桌面全量CI门禁
- 根入口、架构、规格、路线、构建锁、模型矩阵和混合桌面记录移除移动计划/保留规则/断链；桌面历史通过/失败及原计数不追溯改写。README与当前状态补记准确de7732f三格式交付
- 桌面依赖锁、固定模型/模板/文本fixture、`vendor/llama.cpp` Gitlink、packaging、aria2源码/补丁与桌面许可材料无diff

删除合计191个tracked文件，文档删除已包含在此总数，不能与代码专用166重复统计。新增源码边界测试是具体移除范围门禁；文档本身仅做链接、围栏、一致性和空白检查，没有新增形式化业务测试。

## 提交前本机实际检查

| 检查 | 结果 | 边界 |
| --- | --- | --- |
| 严格Python全量`python3 -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'` | 270项，266通过、4既有平台skip，退出0 | 含5项新增desktop-scope检查，不重复累加；新门禁先在旧树出现18条失败，删除后5项全过 |
| `npm test`（`apps/desktop`） | 33文件、701项全部通过，退出0 | 前端逻辑/jsdom，不是Windows原生窗口 |
| `npm run typecheck`、`npm run lint`、`npm run build` | 全部退出0 | Vite build含typecheck；没有安装/升级依赖 |
| actionlint 1.7.12检查全部4份保留workflow | 退出0 | shellcheck/pyflakes未安装的检查不冒称通过 |
| 桌面锁/fixture/上游/packaging/aria2保护路径diff | 无变更 | 上游跨平台来源与许可不为关键词清零改动 |
| 内联Python文档链接/锚点/围栏检查、`git diff --check` | 当前树99份Markdown、415个本地路径链接、19个本地锚点均通过，围栏闭合、无空白错误；退出0 | 只读检查包含两份新增文档，排除上游子模块；不验证外部网页内容或业务运行 |

冻结后独立11组源码/文档边界审查全部通过，无剩余源码或文档阻断；另独立确认99份Markdown、415本地路径链接、19锚点和围栏全通过，旧archive路径零引用，4份工作流actionlint复验通过。

独立复核另重跑scope5项、WindowsCI9项、release27项共41项全通过，均是270项的子集，不累加；另在独立scratch执行7组边界反例，重建移动目录/工厂/Cargo目标/工作流排除或删除早期门禁均被拒，上游锁/许可/fixture平台词仍允许。

实际开发日志和机器变更清单已由任务保存；以上引用最终计数，针对性子集不另加总。npm既有环境配置/新版本提示不是本次升级行为。

## f577a498精确提交原生Windows验证

- 已提交并推送：`f577a49861298ec278293ba2d7231e09c3c07022`；tree `f3ec548cc3049da7e7121d4866bcff07e360dfd4`
- [Actions37313974388](https://github.com/Naza3/Nexa/actions/runs/37313974388)于2026-10-05 13:44:46 UTC成功；[native job111777141797](https://github.com/Naza3/Nexa/actions/runs/37313974388/job/111777141797)、release-identity job111775967120、download-component job111776015819均成功
- Release job111793549071因本次为`codex/dev`分支构建而正确跳过；没有真实tag发布、Release或main合并
- 本节计数直接读取本次native日志与54份脱敏证据库存，`evidence-index.json`的source为f577a498、result为pass；完整独立字节审查随后完成，见下一节；与CI成功分别记录

| 本次Windows检查 | 精确结果 | 计数与验收边界 |
| --- | --- | --- |
| 严格Python全量270项 | 268通过、2个平台skip | 与先前Linux的266通过/4skip分开，不相加 |
| `npm run lint`、`npm run test`、`npm run build`（含typecheck） | 33测试文件、701项全部通过，构建通过 | 不授予应用原生窗口已测试结论 |
| `cargo fmt --all -- --check`；`cargo test --locked --workspace -- --test-threads=1`；`cargo clippy --locked --workspace --all-targets -- -D warnings` | fmt/clippy通过；`windows-rust-tests.log`53组551通过、0失败、7忽略 | 早期选择性transport/store/guard/discovery测试是重复覆盖，不另加总；保留ignored真实含义 |
| 独立桌面壳`cargo fmt`、`cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --target x86_64-pc-windows-msvc`及同目标clippy | 29项通过，fmt/clippy通过 | 独立workspace结果单列 |
| CTest | 4/4通过 | 流、模板、privacy与tool-parser原生用例，不等于生产工具协议已实现 |
| 显式真实模型/host/worker测试 | `windows-real-model.log`3项通过、1项filtered；`windows-real-runtime.log`1项通过；`windows-worker-real-credit.log`1项通过 | 在常规workspace之后显式执行，不把全部7个ignored改算成功 |
| 固定GGUF与应用链路 | 原生输入身份、真实推理、HTTP/CLI、完整runtime/桌面包及解压bridge通过；独立HTTP断连50/50轮通过 | 固定Qwen3-0.6B Q8_0、CPU/2线程/context2048/batch128；不扩大模型/硬件支持矩阵 |
| MSI/Setup三格式 | 同源构建及全部13项安装生命周期通过 | 来源为本次f577a498，旧de7732f结果未转授 |

`windows-desktop/acceptance.json`确认`result=pass`、`package_unchanged=true`、`native_window_tested=false`。`release-manifest.json`绑定f577a498及payload manifest `eb93d6e60144c59ab505582f724922c66866f2e556d5c3abcaf86f69fedc035f`，记录28文件payload、10份许可、3项安装器构建门槛及13项生命周期全部true。生命周期包括install、repair、upgrade、rollback、downgrade_rejected、uninstall、user_data_preserved、running_process_blocked、setup_install、setup_repair、setup_uninstall、setup_exit_codes、setup_wizard；精确job日志明确报告安装验收通过。该CI/staging记录与下节独立产物审计分别成立。

[本次三格式Actions产物](https://github.com/Naza3/Nexa/actions/runs/37313974388/artifacts/11348344567)包含版本0.1.0的portable ZIP、MSI、Setup EXE、aria2对应源码及两份校验清单。产物已生成且独立复核完成；向用户交付另据实际发送记录。安装器与文档收尾提交的来源必须区分：Windows验证覆盖精确f577a498；本节随后仅文档更新不冒称新commit已重新编译或原生验收。文档路径按既有paths-ignore不触发另一轮完整构建，不改变构建安全门槛。用户已接受当前约30分钟流程，暂不优化。

## 本批独立产物审计

本次f577a498的Linux只读独立审计确认2238项断言全部通过、0失败、无剩余产物阻断；其中包含逐文件/逐许可断言，不表示2238个独立业务测试，也不把上一de7732f的2419项计数移用于本批。审计未在Linux执行交付的Windows程序或安装器。

- Setup内嵌MSI与独立MSI逐字节相同；独立解析17张MSI表，与本次源码的封闭authoring匹配；LZX CAB中28文件/42408929字节与便携payload完全一致
- 便携ZIP与独立desktop产物内ZIP相同，嵌套runtime13文件与独立runtime产物相同；三层manifest/hash、PE依赖、大小和库存闭包通过
- 10份许可文件无损恢复746份原文（desktop548、runtime187、download11）；aria2对应源码与独立下载组件相同，三个锁定补丁无fuzz重放后1449份普通源码字节一致
- 主仓库435份普通文件逐一核对Git blob，并按实际CRLF/属性重建Windows源指纹`22051280457d4a05350ce96633e0ce68d4f274dca2620153ccfffa0696fb03c8`，与本次包身份相符；锁定llama.cpp Gitlink保持不变
- 54份原生证据的库存/hash/来源核验通过；13项安装生命周期全部true，独立diagnostics中的69事件与本次精确job日志一致。job日志另含6个早期guard事件，不与69事件重复计数
- 已安装维护时范围/目录覆盖返回0，但两次实际上下文均为USERUNMANAGED `[2]`且目标不变；首次不安全覆盖仍返回1603。没有把返回0误记为拒绝
- inherited-job探针通过；breakaway探针的CI job环境拒绝仍如实保留，不改算成功。独立HTTP报告98项中89通过、9项分层skip，断连50轮全过；skip不包装为全部产品验收完成

| 本次文件 | 字节 | SHA256 |
| --- | --- | --- |
| `Nexa-0.1.0-windows-x64-portable.zip` | 16107805 | `e50649870177418b0529dc223002231ad9b2d161625b3d3d26e975846414ca3f` |
| `Nexa-0.1.0-windows-x64-setup.msi` | 13869056 | `258a4081e8adb0d85dc347ae764abde03687729573c38cc05c01f48b4ab3c069` |
| `Nexa-0.1.0-windows-x64-setup.exe` | 13885440 | `4a5c3cd7401097c5bd9a3e1185f56488b2b624b2b5566ef30d73632b9799a47e` |
| `Nexa-0.1.0-aria2-1.37.0-nexa-corresponding-source.tar.gz` | 5734578 | `c7b0de7b588997704f112b2133a24e585e3f3b1fb5fabcbeae42c77d11fb95cf` |

公开入口为[同次Actions产物](https://github.com/Naza3/Nexa/actions/runs/37313974388/artifacts/11348344567)，另附`release-manifest.json`与`SHA256SUMS`，共六文件。版本仍为0.1.0，不能与先前de7732f的同名文件混淆；以精确commit/hash区分，未覆盖已发布Release。以上字节审计和原生生命周期在本次范围内完成，不扩大下列未验证条件。

## 剩余提及分类与未验证

- ADR0014中的Android用于说明桌面决策背景；ADR0029、本记录及当前状态中的旧路径/标识只说明删除范围，不是当前工程入口
- 独立`Naza3/MNN`仅用于明确不操作的项目边界；上游依赖锁、原文许可与完整llama.cpp内容保留其平台事实
- `tests/fixtures/summary-long.txt`中的两处mobile是固定摘要文本，不是移动代码；更改会破坏实测基线，因此保留
- 通用schema/安全/队列/取消覆盖保留，不以删除产品范围为由删去仍有桌面意义的测试

提交前本机因未提供Rust工具链且llama.cpp子模块未物化，没有执行Rust/原生验证；这一环境限制已由上述f577a498的实际GitHub原生结果补齐，不改写本机未执行事实。安装器仍未签名；目标Windows10/i5-8400、应用原生GUI、Windows Installer ICE、干净机器/离线、两机LAN、长期运行和实际tag发布仍未验。Setup向导CI不等同应用GUI；升级/回滚与3010采用隔离测试fixture，不冒称历史用户版本迁移或生产包触发重启。原始installer build/lifecycle JSON未单独上传，直接证据为严格staging后的发行清单、独立诊断及精确job日志；Linux审计没有重做Windows Authenticode证书链/撤销校验。原生验证和独立审计已完成，不授予上述仍未测试条件通过结论。
