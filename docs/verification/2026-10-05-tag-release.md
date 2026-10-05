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
