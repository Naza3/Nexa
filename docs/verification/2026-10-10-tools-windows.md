# 工具兼容层 Windows 构建与验收跟进

## 已推送的首轮

用户明确要求先 Windows 交叉构建，再更新 codex/dev 并运行原生 CI。最终源码 d5e6b354f17b74c18541adcb3920e39dfbac383e、tree 7aedac5505dca4d1437663b5290988d2acb310d9 已先通过完整交叉门槛，再以 expected old head/非强推方式更新分支。常规 Git 缺凭据，改用已连接 GitHub Git Data API重建相同tree和中文提交顺序，最终SHA重新fetch、干净检出并重跑全部增量门槛；未以旧提交证据替代。

交叉证据：`/workspace/shared/nexa-windows-cross/runs/20261010T024142Z-d5e6b354f17b/`。两个workspace Windows Release/all-targets strict Clippy、10份AMD64静态库、Runtime/worker/acceptance/desktop四EXE实际链接、PE32+与imports/SHA通过；626份源码和前端dist前后一致。额外容量边界测试EXE只编译链接，没有在Linux执行。SDK40个payload SHA与官方目录一致；3个VSIX大小及上级manifest摘要/大小不一致按原记录保留，不声称完整目录签名链通过。

## 首轮原生失败与定位

[Actions 38017934713](https://github.com/Naza3/Nexa/actions/runs/38017934713)于2026-10-10 03:03:58 UTC失败，精确源码d5e6b35。桌面构建、release-identity和同源下载组件成功；runtime-build在真实HTTP/CLI验收失败，后续整包/安装生命周期未执行，无可交付的新完整安装包。

已通过：CTest6/6（含MSVC容量边界）、真实工具两轮1项6.45秒、Windows根Rust694通过/11忽略、Python371通过/2skip、桌面34项、前端1006项、strict Clippy、其他真实模型/取消/调度/worker检查。

[Runtime证据归档](https://github.com/Naza3/Nexa/actions/runs/38017934713/artifacts/11657277810)的ZIP SHA与内部36文件SHA/大小全部核验。HTTP报告为88通过、1失败、9显式skip，50/50断连恢复通过，forced_cleanup=false。唯一失败case为旧 `unsupported_tools`：验收仍在加载前发送 `tools:[]` 并期待400/unsupported_parameter；新协议允许空工具列表。归档没有实际HTTP状态，不能声称失败那次已返回200。

## 修复旧验收，而非回退产品功能

- 将合法空tools正例放在显式加载后，检查200、完整assistant文本、无tool_calls及合法usage。
- 保留unknown field/重复键/token alias负例，增加仍明确不支持的response_format及strict:true；精确核验status/code/param。
- 仅dev依赖生产parser，对真正的验收payload增加契约漂移与判定变异测试；运行中的黑盒oracle保持独立。
- 把windows-real-tools.log纳入受审查证据，但只投影固定测试结果与类型化线程配置；模型正文/失败断言参数省略，源字节hash继续记录。

## 修复后 Linux 验证

xtask两个bin共59通过/1既有忽略，strict Clippy、fmt及diff退出0。严格Python378项中373通过/5平台skip；证据脚本28项为上述子集。

实际构建CLI、worker、xtask后运行 `scripts/run_api_smoke.py`，固定Qwen3-0.6B-Q8_0、50次断连循环，退出0；报告91通过、9显式skip、0失败，正常停止无强制清理。日志与报告保存在 `/workspace/shared/nexa-rust-tooling/tool-smoke-fix-real/`。这证明修复后的真实HTTP/CLI流程在Linux通过，不替代下一轮Windows原生执行。

下一步：冻结修复提交，按相同顺序完成Windows交叉门槛、推送与新原生CI。此记录不把待运行步骤标成成功，main/tag/正式Release未变。


## 最终成功与交付

修复后的精确源码 `e3f2acdb5bb703dfe0e44a5f1a6369007df6a355`（tree `850a7abfba572601d39b1ef6caa503027af90ca0`）先通过完整增量Windows交叉门槛，再非强制更新codex/dev。[Actions38019983165](https://github.com/Naza3/Nexa/actions/runs/38019983165)于2026-10-10 03:51:44 UTC成功：5个构建/验证job通过，分支release正确跳过。

- Windows根Rust702通过/11既有忽略；桌面34项；两strict Clippy通过。
- 前端1006项；Python378项中376通过/2平台skip；CTest6/6。
- 真实HTTP/CLI91通过/9显式skip/0失败，新增strict负例及加载后空tools正例通过；50/50断连恢复，正常退出，无强制清理。
- 真实两轮工具1项6.26秒通过，新增受审查的windows-real-tools.log已归档。
- MSI/NSIS完整17项生命周期全部通过，69事件安装诊断status=pass；两格式安装后package_verified=true、dirty=false，source精确绑定e3f2acdb。

独立下载复核[完整三格式包](https://github.com/Naza3/Nexa/actions/runs/38019983165/artifacts/11658547928)的ZIP摘要、6文件发行库存、便携包28文件、许可与aria2对应源码闭包、安装器身份，以及三份证据内部63文件的hash/大小全部通过；生产 `release_windows.py verify --directory <release> --commit e3f2acdb5bb703dfe0e44a5f1a6369007df6a355 --version 0.3.0` 退出0。

证据：[Runtime](https://github.com/Naza3/Nexa/actions/runs/38019983165/artifacts/11658762058)、[Desktop](https://github.com/Naza3/Nexa/actions/runs/38019983165/artifacts/11658270742)、[Native](https://github.com/Naza3/Nexa/actions/runs/38019983165/artifacts/11658437946)、[安装诊断](https://github.com/Naza3/Nexa/actions/runs/38019983165/artifacts/11658842715)。本地完整归档在 `/workspace/shared/nexa-ci-monitor-38019983165/`；最终精确提交交叉证据在 `/workspace/shared/nexa-windows-cross/runs/20261010T031538Z-e3f2acdb5bb7/`。

本次未签名0.3.0开发测试包已交付，完整artifact55,774,845字节，2026-10-17 03:51:35 UTC过期。MSI SHA256为`8c8243dc40dff8272646e75af1e68a10892da65f348fd4e0cdf2f4022f54dfa5`；NSIS EXE为`1f8017ecca1a736affb75e42d3dc1ac367c75c73ad7baf9a1b02c45b1c6a8af9`；portable ZIP为`574b6769d75c6aa1be88724867bf14e5988baeee92c2d3871bc070fa93c95812`。

未创建tag、发布正式Release或合并main。Windows Server 2022原生CI通过，不替代用户Win10/i5、实际PI Electron GUI或干净离线机器验收。随后仅补记证据的文档提交不冒充这批二进制的源码身份。
