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
