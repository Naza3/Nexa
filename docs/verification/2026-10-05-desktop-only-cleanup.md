# 2026-10-05 桌面源码与文档清理验证

## 范围与前置

任务：按用户要求，在便携ZIP/MSI/Setup安装器交付后移除本项目移动代码与相关文档，后续聚焦桌面。基线为长期`codex/dev`的干净提交`de7732f031c11e44a27f86b33a341c48131a3906`。前置交付来源和全部13项Windows安装生命周期通过事实见[最终三格式记录](2026-10-05-tag-release.md#最终de7732f原生成功与三格式交付)。本记录覆盖其后的未提交工作树，不借用旧包结果证明清理后源码。

范围决策见[ADR0029](../decisions/0029-desktop-only-source-tree.md)。没有修改独立`Naza3/MNN`仓库或重写Git历史；没有操作tag、Release或main。

## 实际变更

- 删除166个移动工程专用tracked文件：验证器、独立移动workspace、MNN适配/探针/补丁、专用脚本、专属许可材料和两份Android CI
- 删除25个专用文档：旧跨端快照11份、移动计划/契约/验收5份、ADR0008–0013六份、移动验证报告3份
- 共享代码移除`RuntimeConfig::android()`与开发验收报告的`android_arm64_cpu`占位；零/一等待槽调度回归保持显式fixture，模型平台拒绝回归改用未支持的Windows arm64值，通用流式导入保留
- Windows工作流移除移动路径忽略项；新增5项项目源码边界检查，保持桌面全量CI门禁
- 根入口、架构、规格、路线、构建锁、模型矩阵和混合桌面记录移除移动计划/保留规则/断链；桌面历史通过/失败及原计数不追溯改写。README与当前状态补记准确de7732f三格式交付
- 桌面依赖锁、固定模型/模板/文本fixture、`vendor/llama.cpp` Gitlink、packaging、aria2源码/补丁与桌面许可材料无diff

删除合计191个tracked文件，文档删除已包含在此总数，不能与代码专用166重复统计。新增源码边界测试是具体移除范围门禁；文档本身仅做链接、围栏、一致性和空白检查，没有新增形式化业务测试。

## 本机实际检查

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

## 剩余提及分类与未验证

- ADR0014中的Android用于说明桌面决策背景；ADR0029、本记录及当前状态中的旧路径/标识只说明删除范围，不是当前工程入口
- 独立`Naza3/MNN`仅用于明确不操作的项目边界；上游依赖锁、原文许可与完整llama.cpp内容保留其平台事实
- `tests/fixtures/summary-long.txt`中的两处mobile是固定摘要文本，不是移动代码；更改会破坏实测基线，因此保留
- 通用schema/安全/队列/取消覆盖保留，不以删除产品范围为由删去仍有桌面意义的测试

当前执行环境未提供Rust工具链且llama.cpp子模块未物化；本批Rust test/fmt/clippy、原生Windows构建、真实模型和安装生命周期尚未执行。必须由后续精确新提交的完整原生CI验证，不把de7732f的551项Rust/13项安装结果复制为本批通过。目标Windows10、应用GUI、干净机器/离线、两机LAN与长期验收仍独立待验。
