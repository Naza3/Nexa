# 2026-10-06 模型移除与聊天 Markdown 验证

## 基线与范围

既有正式版为 main `5c26e34aa74bf5552b9455e65f42883acbd61d6f` 的 v0.1.0。开发分支原为 `f4318317464a71b0b08f03accf477c788a046160`，仅多两份验收记录；本批先同步 main 历史后实施，没有改动正式标签或公开附件。

用户要求模型库列表移除，以及聊天 Markdown 展示。合同见[ADR0030](../decisions/0030-nondestructive-model-unregistration.md)和[ADR0031](../decisions/0031-safe-chat-markdown.md)。不加入磁盘删除、其他模型自动卸载、任意链接导航、图片联网、工具执行或新的推理后端。

## 模型移除

通过原始模型 ID 与列表 generation 确认，前端仅作预检，在线 actor 再原子检查繁忙/驻留状态。schema3索引同文件保存抑制登记与显式恢复；只读列表与服务重启保持隐藏。外部文件、受控副本及manifest、参数档案、历史测试与聊天原文均保留。停止实例路径只检查元数据，不全库hash，不启动服务、不初始化凭据。

独立反例覆盖源目录失踪、硬链接/多别名精确恢复、同ID冲突、提交前失败、旧确认和刷新失联。发现的Faulted状态卸载指引无效已修正为用户通过左侧服务按钮显式停服；不会自动调用停止或取消。

## Markdown 与许可

精确新增 `react-markdown 10.1.0`、`remark-gfm 4.0.1`；旧依赖版本未升级。仅assistant回复渲染，用户输入、请求与历史保持原文。AST白名单、图片占位、无href导航、代码/整条原文复制、流式围栏与单消息异常回退均覆盖。

独立实测发现4096层引用可令上游解析器抛出RangeError，已用单消息错误边界回退纯文本并验证后续恢复。复制代码去掉解析器额外合成的换行，保留实际文本；不将模型正文写入诊断。

新嵌套npm生产依赖的原始安装路径含node_modules，被既有许可原路径安全校验拒绝。收集器改用完整lock location的SHA256作为安全输出目录，原库存保留精确lock_location、component/version/source/integrity与原文字节；不放宽安全校验。永久回归覆盖同名不同版本与嵌套scoped包。

真实生产依赖107包，收集109许可与1份原库存，110原件逐字节恢复；另用已验证旧包作只读基料重建根许可层，根651/runtime187/download11份原件逐字节闭合，最终仍10份独立许可文件。此组合实验不是本轮Windows新包已构建的证明。

## 开发环境实际检查

- Rust1.98.1，model-store/runtime-core/runtime-api/desktop-bridge四crate all-targets：18组381通过、0失败、1既有忽略；相关clippy `-D warnings` 与fmt通过
- 最后Store::list整锁调整：model-store88项和clippy复验通过；不是另加88个产品用例
- 独立桌面壳Linux31项通过，clippy/fmt通过；Windows专有命令体需原生CI
- 最终前端36文件790项通过，typecheck/lint/Vite build退出0。约601kB JS/gzip179kB，保留Vite大chunk提示，无构建错误
- 官方npm registry全量audit（含dev）退出0，五档漏洞计数均0；仅只读查询，没有自动修复或升级旧依赖
- 严格Python全量271项：267通过、4既有平台skip；新增命令ACL精确集与嵌套许可无损回归通过
- 独立模型移除Rust5项、前端7项，以及Markdown50项反例全部通过；其中相关永久回归与全量检查重叠，不重复累加

## 原生与交付边界

本记录为开发验证。精确提交后的GitHub完整Windows构建、真实GGUF/HTTP/CLI/独立解压桌面与全部MSI/Setup安装生命周期仍待运行。没有用已发布v0.1.0的CI证据替代本批结果。目标Win10原生窗口、干净机器/离线、长期运行等仍独立待验。

新schema3索引的版本回退限制见ADR0030；既有公开v0.1.0和其附件未被替换。此批不自动创建新发行标签。
