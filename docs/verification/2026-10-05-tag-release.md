# 2026-10-05 Tag 自动发行开发验证

## 范围与身份

- 任务：GitHub tag 自动构建便携 ZIP、MSI、Setup EXE，并在同一 Release 提供校验与对应 aria2 源码
- 源基线：长期 `codex/dev`，`9a3de0317129d0c09f6986a0e758022f2f83ea21`
- 本记录仅覆盖本轮版本/发行脚本、工作流及其开发验证；安装器实现与 Windows 原生运行另记
- 未创建 tag/Release、未合并 main、未改变仓库安全设置，不把本地测试的模拟 GitHub API 当作真实发布

## 已执行检查

1. `python3 -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_release*.py'`：25 项通过，退出 0
2. `python3 -m py_compile scripts/release_windows.py scripts/release_version.py`：退出 0
3. PyYAML 6.0.3 读取工作流：四个 job 为 release-identity、download-component、native、release，结构通过
4. 官方 actionlint 1.7.12 Linux amd64 发布二进制按同一官方 release 的 checksum 验证后运行：`actionlint -shellcheck='' -pyflakes='' .github/workflows/native-windows.yml`，退出 0；未把未安装的 shellcheck/pyflakes 宣称为通过
5. `git diff --check`：退出 0
6. 安装器作者完成 CLI/报告后，`python3 -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'`：全量 247 项，其中 243 通过、4 个既有平台 skip，退出 0；发行 25 项及安装器静态 10 项是其中子集，不另加总
7. 新 `zip_inventory` 对已有原生 Windows `9a3de03` 便携 ZIP 实际读取校验通过：28 文件、manifest SHA256 `2f40e2723dc1c1265023f715ab82bf19d4ad39040bbd8d016bcd9458ed886043`；未改该 ZIP 字节，未把它称为新安装包

## 反例门槛

- tag/版本只接受规范稳定三段；拒前导零、后缀、注入字符串、非 ASCII 数字、MSI 数值溢出与保留版本
- root Cargo、独立壳 Cargo/Tauri、npm/锁文件及本地 crate/锁版本必须一致；精确 checkout SHA、干净源码和 tag 版本一致
- portable ZIP 内全部文件和 SHA256SUMS 闭包、嵌套 runtime/download 身份、同字节对应源码；禁止缺失/追加/更改文件、路径穿越、大小写别名和非规范路径
- 独立审查发现 WindowsPath 排序与 CRLF 不同于 Linux；已改为严格唯一的 hash 映射，兼容整份 LF 或 CRLF，拒混合换行、重复/空/错误记录，新增回归并用真实旧包复验
- MSI/Setup 报告必须绑定相同 commit、version、payload manifest 及两个安装器 hash；13 项生命周期检查必须显式全部为 true，不接受空 checks、未审新字段或整数代替 bool
- 分支、workflow_dispatch、PR 无法发布；已有公开 Release 不增删/替换资产；现存不匹配 Release 或附件拒绝
- 新版本 draft 完整上传、服务端 digest 与封闭库存核验后再发布；只恢复完全相同身份且已存字节一致的 draft
- 缺失或被移动的 tag 不创建/改写；同时覆盖轻量及附注 tag 的精确 commit 解析
- token 只进入独立发布 step，HTTP 客户端不跟随携带 token 的重定向；全部第三方 Actions 保持完整 SHA 固定

## 未验证与下一步

本机为 Linux，本记录没有执行新的 MSI 数据库、原生 Setup、安装生命周期、Windows 原生构建或实际 GitHub Release。安装器作者与独立审查完成后由主代理统一提交，在公开仓库标准 `codex/dev` 构建中验证三格式；该分支构建不发布 Release。实际 tag 发布须在维护者确定版本和指向提交后进行。

Windows 10 目标机器原生窗口、无开发工具/离线及长期稳定性仍单列，不能由 Server 2022 CI 或 Setup 自动向导控制流替代。

## 提交前收口

用户已合并PR8，开发分支快进到main的4c40a0d（tree与9a3完全相同），工作树改动保留。最终严格Python247项：243通过、4既有平台skip。独立发行25项、安装器10项及33项作者表/真实旧ZIP检查通过，均属针对性证据而非额外加总。GUI用例已修正为同时等待Finish文字和启用状态。独立审查结论为可进入原生验证，无剩余已知源码阻断；真实MSVC/Windows Installer/向导与13门槛仍待精确提交CI，ICE与用户Win10视觉未执行。

## 首轮 Windows 构建与生命周期失败

精确提交 `51d2d506a0bc4888b84bab5fa7043a52806d4dac` 的 [Actions37281440005](https://github.com/Naza3/Nexa/actions/runs/37281440005) / [native job111671630727](https://github.com/Naza3/Nexa/actions/runs/37281440005/job/111671630727) 中，应用、真实原生链路及完整便携桌面验证通过，MSI/Setup 实际构建成功。对应 payload manifest SHA256 为 `7d53903cf055f648d28289191d9e3e8e9f3974e293d948f2dd9c8c7a269576e4`，MSI 为 `68efd2915847f21d5932553674f561354bc259b07c8f1d29693884c050cfdc9a`，Setup 为 `b031e911b7a7450eda4322840ff9a328b658f3cbc7a640655422b04c3c6d6d0d`。

随后安装生命周期在有界 240 秒等待后失败；现有上传证据没有包含生命周期报告，尚不能由日志推定具体阻塞步骤或窗口。整体构建未通过，三格式 Release 未发布。这些安装器构建 hash 是失败 run 的诊断身份，不是通过生命周期的新交付。

下一修复保留原13门槛，加入受限阶段/退出码/动作/窗口类别的独立诊断 sidecar，并让工作流即使失败也只上传通过封闭 schema 校验的该脱敏报告。缺失输入仅标记缺失，校验失败不上传；不会将原始build/lifecycle报告直接加入白名单。原始 msiexec 日志、任意窗口标题/控件文字和用户路径不作为公开 artifact 上传。

### 失败诊断保留的本地复验

新增工作流 sidecar stage/upload 契约以及独立 UNVERIFIED 安装器 artifact 契约两项测试。最终当前工作树严格 Python 全量 255 项：251 通过、4 个既有平台 skip，退出 0；发行 27 项、安装器静态 10 项和诊断 6 项均在其中，不另加总。诊断测试实际覆盖 CLI 缺失输入标识、未知/重复/非有限字段拒绝、路径/自由文本过滤、软链拒绝、失败时零部分输出及不强杀事务进程。新版工作流 actionlint 1.7.12 和 `git diff --check` 再次退出 0。

失败时另行保存的三文件安装器 artifact 明确标为 UNVERIFIED，仅用于解析正式 MSI 数据库和 Setup 资源，不能作为验收通过或正式交付的依据。它与受审诊断 artifact 分开，Release job 不消费；没有放宽原13项生命周期、版本/源码/许可或发布门槛。本地检查不能代替下一轮原生 CI。

## 第三轮系统门禁边界与隔离修正

第三轮37291142335（2bbd72d）确认默认MSI入口在NexaGuard的guard_os提示等待240秒，前面的wizard取消/非法参数/未安装卸载通过；版本查询具体内部原因仍未知。候选修正移用带兼容manifest的独立只读EXE、MSI Type2强制同步返回码，并保留目录/进程guard；新增长构建前真实msiexec只读探针及旧版本查询数字诊断。严格Python259项：255通过/4既有skip；独立20项installer/diagnostics、27项release、20项OS契约、13项诊断反例通过（子集不加总）。新实际安装、全部13项门槛与目标机仍待验，未放宽超时或绕过检查。

## 第四轮实际安装通过后的维护范围修正

第四轮37300524739（4067823）实际通过默认安装、MSI/Setup维修、alias目录维修及busy进程阻止，旧OS查询数字为6.3/build20348/error1150，新manifest EXE门禁有效。维修ALLUSERS=1返回0使旧硬编码断言失败；新测试必须通过官方API读出唯一USERUNMANAGED上下文并复核原路径/字节/数据和无机器/外部目录，不能只放宽返回码。首次安装不安全覆盖仍强拒绝；3010私有fixture重新生成PackageCode。严格265项：261通过/4既有skip；独立20项不变量、26项installer/diagnostics/context及27项release通过，均为子集。真实新上下文API与剩余13门槛待下一轮native，不提前宣称MSI/Setup交付。

## 最终de7732f原生成功与三格式交付

精确提交 `de7732f031c11e44a27f86b33a341c48131a3906`，tree `afde24f2d64c81cc7e4484c6c8641c7ed87a8fe1` 的[Actions37306309927](https://github.com/Naza3/Nexa/actions/runs/37306309927)已成功。以下更新上述各轮“待验/失败”后的最终状态；旧失败记录保留，不将它们改写为成功。

### 实际Windows验证

- 主机为Windows Server 2022，分支构建；原生应用、固定GGUF、HTTP/CLI、桌面bridge、下载与完整包门禁通过
- Rust证据为53组551通过、0失败、7忽略，桌面壳29通过，CTest 4/4通过；这是该精确源提交的运行结果，不覆盖随后移动清理
- 正式MSI/Setup构建三项门槛及全部13项生命周期为true：安装、修复、升级、回滚、降级拒绝、卸载、用户数据保留、运行进程阻止、Setup安装/修复/卸载/退出码/向导
- 默认MSI Basic UI安装、MSI/Setup删文件修复、短路径修复与真正已安装runtime启动通过；忙进程阻止维修/卸载后由应用API正常停服，没有通过强杀绕过
- 首次不安全ALLUSERS/目标目录覆盖返回1603；已安装维护请求返回0，但官方API确认唯一USERUNMANAGED上下文`[2]`，无machine/managed登记，路径/字节/数据哨兵与外部目录不变。此为安全归一化，不能把0记作拒绝
- Setup向导三操作Next/Back/Cancel及实际Apply/Progress/Finish执行通过；退出码0/87/1602/1603/1605/3010均有真实观察。升级/失败回滚与3010使用隔离的私有测试fixture，不进入交付
- 原生breakaway探针因CI job环境拒绝spawn（错误5），如实保留；inherited-job探针和产品生命周期通过，不将前者改算成功

### 独立产物核验与交付

Linux只读独立审计最终命令 `PYTHONDONTWRITEBYTECODE=1 review/venv/bin/python review/supplement.py` 退出0：2419项断言通过、0失败。含逐文件、逐许可和来源检查，不是2419个独立产品测试，也不是在Linux上执行Windows安装器。

| 文件 | 字节 | SHA256 |
| --- | --- | --- |
| `Nexa-0.1.0-windows-x64-portable.zip` | 16107773 | `b2398432a1f1a17b12429e37614072d3f0e4060ddf2ea6f8e9aadaca99b0bdbb` |
| `Nexa-0.1.0-windows-x64-setup.msi` | 13864960 | `0168d898f2862ac62f1b0276e9303ac79c4c3b9f81abaa9df8e472ac317ed458` |
| `Nexa-0.1.0-windows-x64-setup.exe` | 13881344 | `02541bb73eb199ddfdc1f5919e21f8afa0d293944479dbd4791afad2e7be5bfc` |
| `Nexa-0.1.0-aria2-1.37.0-nexa-corresponding-source.tar.gz` | 5734590 | `35385ae26a78b6253e090fd6db592e3f5e0abbea5f1ac3ee0d78b462ee85a066` |

发行集合另含`release-manifest.json`与`SHA256SUMS`，共六文件；payload manifest SHA256为`1daa58b4244d3c2045d2b58a563673690b86d2a31aea4d423cb88f55e93f6b57`。三格式及校验/来源说明于2026-10-05 12:49:24 UTC交付，发送不等于用户已在目标机安装。

- 便携ZIP与单独desktop artifact内ZIP相同；Setup内嵌MSI与独立MSI完全相同；独立解出的MSI LZX CAB全部28文件/42408941字节与便携payload相同
- 17张MSI表独立解码并与封闭authoring逐行匹配；HKCU/每用户/x64范围与固定安装路径成立，没有生产fixture或模型/配置递归删除
- 10份许可文件完整恢复746份原文（桌面548、runtime187、download11）；许可归属/hash、aria2对应源码与1449份补丁重放源码均匹配
- 主仓库623份普通文件逐一匹配Git blob；按实际CRLF/属性重建的Windows来源指纹`681da572eed6420ed3244d77f0cfb9c8c37cc9dc9ac0549cbb320a13a012a646`与原生清单一致。上游Gitlink核对通过，本机审计未独立重编译未物化的子模块
- 54份原生证据库存/hash/source核对通过；独立诊断69事件与精确job日志逐字段匹配，另有6个早期MSI guard上下文事件

### 完成边界

[PR #9](https://github.com/Naza3/Nexa/pull/9)仍为草稿，main为`4c40a0d0f969dfb6d10a295bb7b7922f741c9643`。分支构建的Release job按条件跳过，清单中的`v0.1.0`仅为版本身份，未创建/推送实际tag或发布GitHub Release。

交付安装器未签名，Windows Installer ICE未执行。两份原始installer build/lifecycle JSON未单独上传，可直接取证的是严格staging后的发行清单、诊断和精确job日志。升级/回滚采用同payload私有下一版本fixture，不证明历史用户版本迁移；3010不表示正式生产包主动安排重启。Server 2022与Setup向导通过不代替用户Win10/i5-8400、应用原生窗口/选择器/剪贴板、两机LAN、干净机器/离线和长期稳定性。独立审计未重做Windows签名/撤销检查，仅核对原生证据与交付字节绑定。

安装器任务在上述范围内完成后，移动源码清理作为[独立任务](2026-10-05-desktop-only-cleanup.md)开始；旧交付包不改写为清理后的新来源。
