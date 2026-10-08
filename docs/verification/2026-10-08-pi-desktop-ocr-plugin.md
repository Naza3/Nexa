# PI Desktop OCR 插件验证

日期：2026-10-08。任务 W04-PI-OCR-1。源码/安装包及 Linux 分层验证已完成；Windows Electron 安装和用户 i5-8400 目标机仍待验证。授权来自用户“先完成这个插件”；期间关于现成插件的提问作为方案核对，不取代实施目标。

## 范围与环境

新增 `integrations/pi-desktop-ocr` 独立0.1.0包；Nexa 仍0.2.3，未修改 Rust/native/Tauri/HTTP协议，未创建tag或推送。基线 `a18e9a184797519d66e23aba155ecd9988672dc0`，`codex/dev` 已包含本轮读取的远端main。

PI Desktop 固定 `779e16d9c3ca2e966a7ae3db9dd0707243a2831f` / 0.17.0；官方 SDK/devkit 由同版本源码构建，未冒用公开npm同名包。Node24.19.0、npm11.9.0；依赖与许可见插件 lockfile/THIRD_PARTY_NOTICES。Side Chat外观适配参考固定 `a815de103f3f28f6bbdbe4824753ad761d393284`，原MIT许可随包保留。

## 实际命令和结果

下列插件命令在 `integrations/pi-desktop-ocr` 执行，命令均退出0；完整环境证据位于 `/workspace/onboarding/pi-ocr`，不会把模型、令牌或识别正文加入源码提交。

| 检查 | 实际结果 | 环境证据 |
| --- | --- | --- |
| `npm test` | 52通过、0失败/忽略；客户端34、controller/storage12、图片6 | `plugin-unit-tests-final.log` |
| `npm run pack` | 官方devkit成功生成14文件安装包；生产CJS/renderer bundle及完整许可 | `plugin-pack-delivery.log` |
| `bash /workspace/onboarding/pi-ocr/install.sh` | 冻结lock安装、重新build、官方check通过 | `install-refresh.log` |
| `NEXA_OCR_PLUGIN_PATH=<解压目录> node --test /workspace/onboarding/pi-ocr/nexa-plugin-host.test.mjs` | 最终包官方真实child的11项宿主契约通过 | `nexa-plugin-host-deliveryfinal.log` |
| Chromium + `renderer-controller-browser.py` | renderer→真实Controller/Store，49桥调用、12图片分片、两图队列；视图/提示词/预算TOML重建恢复，历史/复制/真实导出文件断言通过 | `renderer-real-controller-evidence.json` |
| `node /workspace/onboarding/pi-ocr/real/run-host-real-final.mjs` | 官方child加载生产main，真实Nexa/worker/GLM-OCR两图成功、TOML重载及自有服务正常关闭 | `real/host-real-final-{result,cleanup,package}.json` |

官方check警告保留并解释：本机网络/写文件为需用户授权的权限，127.0.0.1确实是所需目标；clipboard.write静态扫描仅查看main而调用实际在renderer，真实浏览器已检查复制。未为消除提示放宽网络目标或去除必要权限。

## 可验证行为

客户端测试覆盖同socket身份验证、实例变化、忙/故障、加载所有权、SSE分片/UTF-8/CRLF/终态、超时取消、未确认清理、输出上限、指标身份匹配；无正文重放。宿主测试覆盖插件生命周期、拒绝网络授权时零HTTP、令牌不进snapshot、记住/忘记、分片预算/尺寸、相对路径导出、缺ID不得清历史、损坏TOML错误不得携带正文或凭据、卸载后拒绝新设置等。

Chromium宽度1100/700/390/300及深色外观通过早期布局检查；后续真实Controller/Store桥检查1100/390截图、浏览器与controller错误均0。推理和PI服务在此浏览器检查中是明确夹具，不算真实模型结果。3056×5812原图因超过16Mi像素被拒；显式最长边4096后2154×4096通过；另验随机大PNG的多分片及EXIF方向JPEG。

真实模型检查使用两张960×300合成小票，按 `02-先导入.png`、`01-后导入.png` 先后上传，活动顺序保持；各3/3固定锚点、385输入token/36输出token、finish=stop。总36.581秒，首图加载后prefill/decode，第二图复用模型；各自性能和两条非空history保存，重新加载TOML后保持。4线程/context8192/batch256，首图prefill16.748419秒/decode1.085404秒，第二图16.440058/1.128651秒。只说明该Linux云机器/合成小图行为，不说明用户长表质量或Windows CPU速度。

该真实流程使用未修改的官方plugin child，父API broker为夹具，Nexa与原生worker/模型为真实进程。Nexa stop与进程退出码均0；child在收到unload ACK后由夹具结束IPC进程。令牌由自有fixture实例产生，未写入插件私有文件；重载后hasToken=false。

## 发现与修正

- GLM-OCR内容顺序应先image_url再text；最初真实测试失败保留，修复后真实识别成功。
- renderer旧枚举source与主进程text不一致，早期宽松mock未发现；真实Controller桥发现并修复，TOML重载验证通过。
- TOML解析错误包含源行，已统一为固定错误；补损坏凭据/正文不泄露测试。
- 删除历史缺ID、设置prototype键、凭据临时文件清理、关闭后新变更入口均已补强并验证。
- 真实fixture的模型启动hash等待从10秒改为120秒，生产插件流程尚未开始时的fixture失败不误报成产品失败；旧证据保留。

## 最终交付与边界

`integrations/pi-desktop-ocr/dist/io.github.naza3.nexa-ocr-0.1.0.piplug`，334150字节、14文件，ZIP与staging逐字节复核通过；清单 `deliveryfinal-package-manifest.json`。SHA256：

```text
12f170a11169b41b5762a0c1f5ac767f01725a3d426bc901012e5823bb3f0f8b
```

真实模型复验包原SHA为 `4dfc4fda46b7248f4dbf2066b4a8566c3aa1e89f9c19126ea2945a619aee757d`；随后只纠正包内README的安装菜单文字，最终产物的可执行文件逐字节保持。已验main SHA256为 `6e99d1b41986f302e98f74c588e617c8c3c69733399b08e734661cc64fb095df`。安装步骤见[插件README](../../integrations/pi-desktop-ocr/README.md)。

Windows原生PI Desktop安装/iframe桥/目录选择和用户i5设备长图待验；未以Linux宿主fixture声明这些通过。没有本轮Nexa二进制改动或远端构建，因此未重做Windows交叉编译；后续任何触发CI的推送仍先遵守AGENTS的Windows交叉门槛。

云环境已实际刷新并验证插件开发依赖，保留官方devkit、宿主与真实模型fixture；已追加保存install_script/start_skill草稿，保留现有Windows交叉规则及工作区。保存草稿不是发布新快照，也不假定服务跨任务存活。下一步是用户在Windows PI Desktop安装此包，连接已登记GLM-OCR的Nexa，验证实际图片；失败依据具体界面错误继续修复。

## GitHub公开下载交付

同日用户指出云端路径无法下载，并明确同意公开发布。本轮任务W04-PI-OCR-RELEASE-1完成独立插件预发布；不把本节发布事实追溯为上一节未执行的Windows宿主验证。

- 标签 `pi-ocr-v0.1.0` 固定至插件已验源码 `93cdf6d4493570714f744cd7c246ccc908a4a8d3`。此标签不匹配既有Windows `v*` 触发器，初次推送未启动主程序CI。
- 普通上传HTTP400；确认已有GH_TOKEN绑定后正常使用官方 `gh release upload`，uploads.github.com 返回401 Bad credentials。Release API仍可正常创建/修改草稿。未读取或输出凭据，没有反复盲试或要求用户再发密钥。
- 新增固定用途 `.github/workflows/pi-ocr-release.yml`，提交 `0f0e0fb67d1542d6ab3fcb8ab5b3196270e52852`。仅专用发布分支运行：只读job固定源码/PI devkit、执行52项测试/官方打包并核对完整包SHA；写job用Actions自身令牌向固定草稿406423946补齐附件，不覆盖、不自行公开。
- 按用户既有要求，在推送此工作流之前，`bash /workspace/onboarding/windows-cross/build.sh` 退出0；证据 `/workspace/onboarding/windows-cross/runs/20261008T045644Z`。该干净源码的前端、原生库、两workspace Windows strict Clippy、四EXE链接/AMD64身份及源码收据检查全部通过。未在Windows运行EXE。
- 仅把该提交推送到 `codex/pi-ocr-publish` 以隔离主程序CI，不创建worktree、不改main、不移动旧tag；远端codex/dev保持原状。本轮隔离为附件发布故障恢复，不取代长期开发分支安排。
- [Actions37729949673](https://github.com/Naza3/Nexa/actions/runs/37729949673)的build/upload-draft两job成功。云端重打包334150字节与本地已验包SHA完全一致；官方devkit的ZIP日期固定且文件排序稳定。固定草稿查询用release ID；按tag查草稿实际404已在推送前修正。
- 核对已上传的包和sha256文件后，API公开为pre-release且 `make_latest=false`。发布页 [pi-ocr-v0.1.0](https://github.com/Naza3/Nexa/releases/tag/pi-ocr-v0.1.0)，附件仅插件及校验文件；旧Nexa发行保持。
- 实际读取两个公开 `browser_download_url` 均HTTP200，逐字节等于本地文件，SHA与GitHub asset digest一致。包SHA仍为本文的 `12f170a...3f0f8b`。完整本机证据 `github-plugin-release.json`、`github-plugin-download-verification.json`、`github-release-action.log` 位于 `/workspace/onboarding/pi-ocr`。

最终[安装包直链](https://github.com/Naza3/Nexa/releases/download/pi-ocr-v0.1.0/io.github.naza3.nexa-ocr-0.1.0.piplug)可从用户本机浏览器下载，替代此前只能打开云端文件的workspace链接。插件安装位置和Nexa令牌导入步骤仍按README；Windows Electron与用户CPU实际效果待验。
