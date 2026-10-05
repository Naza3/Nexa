# 桌面控制与本机网卡发现验证

**最新安排：** 用户2026-10-04 13:28 UTC明确恢复提交与GitHub Actions构建。本批四项修复统一在codex/dev交付；下文“暂缓”是此前检查时的状态。源码验证通过不等于Windows原生构建或新包交付成功，后续以精确提交CI结果为准。

日期：2026-10-04。分支：`codex/dev`，基线 `996d5e0d9a2c5466e3140563f61bd3e447fc720d`。状态：源码实现与本地联合验证完成；用户暂缓构建，本报告不能代替新源码的Windows构建结果。

## 范围

- 模型添加的完成、部分完成、失败、取消结果可以显式关闭；进行中、停止中或恢复未确认时不隐藏进度。关闭不调用模型API、不删除登记/测试证明，也不撤销后台合法刷新
- LAN设置从本机只读枚举当前网卡的私有IPv4，显示名称和地址、可刷新和手动填写；不自动启用、保存或放宽客户端范围
- 左导航栏底部统一启动/停止服务主控，显示服务状态，过程防重入；继续通过已有停止确认，避免无提示中断其他应用请求
- HTTPS暂缓；不改变推理引擎、模型准入、防火墙、独立LAN凭据或回环管理边界

## 已完成的分层检查

| 检查 | 结果与限制 |
| --- | --- |
| 添加结果关闭切片 | typecheck、lint、445项前端测试、build通过；独立5文件125项回归通过（为445子集，不累加） |
| 命令权限封闭清单 | `runtime_lan_addresses`加入主窗口白名单与精确测试，11项Python桌面包测试通过 |
| 严格Python全量 | `python3 -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'`，194项，190通过、4项Linux平台跳过 |
| Rust联合回归 | 全workspace/all-targets 40组472通过、0失败、7既有忽略；完整clippy/fmt通过（Linux） |
| LAN枚举独立复核 | 最终8项定向测试及Linux strict clippy通过；Windows bridge all-targets check/clippy、壳交叉check通过。5项Windows专用测试尚未原生执行 |
| 合并后前端 | typecheck、lint、23文件508项测试、production前端build及diff检查通过；独立8文件190项子集和typecheck通过，独立审查无剩余阻断 |
| 原生Windows | 用户要求暂缓构建，尚未执行本批Windows原生测试或生成新包 |

本地日志：`/tmp/nexa-add-dismiss-20261004/frontend.log`、`/tmp/nexa-add-dismiss-review-20261004.log`、`/tmp/nexa-lan-ui-package-tests.log`、`/tmp/nexa-lan-ui-python-full.log`。日志为开发环境证据路径，不进入分发包。

用户13:01反馈API缺失模型ID的400错误，明确暂缓构建。本批尚未提交推送，不触发Actions；只读诊断与本地回归继续，不能将以下验收清单理解为已发新包。

前端最终日志：`/workspace/shared/nexa-ui-final-validation.log`（SHA256 `38bf0ced1766dfa415aecacc4f645996aef7ef21a19a7ef8d1aa6410ea76d50b`）。

Rust联合日志：`/tmp/nexa-desktop-controls-rust-full.log`；独立枚举日志：`/tmp/nexa-lan-addresses-review-{focused,linux-clippy,windows-check,windows-clippy}.log`。

最后统一三处初始化指引为“左侧服务按钮”。该纯文案修订后，typecheck/lint和508项全量测试再次通过。npm更新检查访问registry受到环境网络策略阻断，组合命令未完成构建；随后直接使用已安装本地 `./node_modules/.bin/tsc --noEmit && ./node_modules/.bin/vite build` 完成离线前端构建，退出0，没有重试被拒的网络请求。补充日志 `/tmp/nexa-controls-final-copy-full.log`；本地直接构建输出chunk `f34456`。没有执行Windows打包或触发CI。

## 目标Windows手动验收

1. 打开模型、设置、下载等页面，侧栏底部均可找到服务主控；窗口缩小和系统缩放后控件不被导航内容覆盖
2. 服务已停止时显示启动；点击一次后显示进行中且重复点击不重复启动；成功后显示停止与运行状态
3. 有调用任务时点击停止，应先看到已有确认；取消保持运行，确认后才执行停止；失败/未知状态不显示虚假已停止
4. 添加一个模型，结束后关闭结果卡，模型仍保留；新选择/新任务不能被旧结果的延迟事件清除；进行中仍可看进度并按现有入口取消
5. 在LAN设置刷新网卡，核对名称和IPv4；多个网卡可明确选择，VPN/虚拟网卡不被标为已验证物理LAN；没有候选或读取失败仍能手填
6. 已有未保存输入时再次刷新，输入不被覆盖；选择地址不自动启动或保存服务，也不自动设置允许客户端CIDR
7. 断开/更换网络后刷新检查；保存仍要求先停止服务。确需开启LAN时沿用[局域网验收](../lan-api-usage.md)，两机可达性不能由候选列表或本机测试推断

Windows原生窗口视觉、用户Windows10/i5-8400/16GB实际体验和两设备局域网验证独立记录。Linux/jsdom或交叉编译不等于这些场景已通过。
