# 2026-10-04 模型管理与 LAN 整合验证

用户反馈平行分支冲突，关闭 PR #6 后要求开发方处理，并明确后续长期维护一个开发分支。当前在 `codex/nexa-add-model` 将 main `26206ef882e0d47d506767dd683aa32e20da111d`（已含LAN）合入 Add `206d965cb9a40c61b94b8fe8cb9c3ea3821eb7a2`，不是修改main，不强推，也不让用户逐项选择冲突侧。

## 合并检查点

真实Git合并有6处文本冲突：项目索引/状态、执行规格、架构、controller imports、打包壳契约测试。逐项保留两边语义；不能使用整体ours/theirs丢弃另一功能。Rust自动合并的6个交叉文件另做独立语义审查；33个Tauri命令在注册、build与ACL中严格一致。

实际exit0：

- 全workspace/all-targets：40组437通过、0失败、7既有忽略
- 全workspace clippy `-D warnings`、workspace/Tauri fmt、diff检查
- 前端typecheck/lint、16文件291项测试、production build
- Python scripts155项：153通过、2平台skip
- 独立bridge/model-store13组171项、Linux壳31项、打包契约11项，均为各自回归子集，不重复累加

前端291项包含合并后原284项与新增7项交叉测试，不能将原单片247和230直接相加。新增覆盖LAN保存与Add选择/提交/取消/关闭互斥、晚回picker租约释放、恢复后LAN操作及Modal取消。

独立临时harness核验Add持work时LAN保存/复制拒绝，共享InstanceLock保留选择，schema2无默认目录模型仍可浏览，保存idle/LAN不覆盖模型库，未选payload不登记，保存配置不生成LAN key。未绑定任何服务。该审查harness初次误链接另一Tokio产物导致无reactor，按desktop_bridge fingerprint修正依赖后exit0；不是产品失败，不隐去这次审查环境错误。

LAN独立token/router、48+16连接预算、actor loaded-only、本机proof及锁内配置重读保留。Add多来源schema2、opaque租约、定向登记、configure-only、默认不扫描及取消/保存事实均保留。

## 仍待完成

Windows本机测试记录路径修复、结果/按钮反馈，以及文件校验超时和不自动卸载设置，在同一开发分支继续按序实现；本检查点不包含这些改动。最后必须再做完整联合回归、Windows构建和新PR。

未运行真实Windows原生选择器、剪贴板、窗口、实际两设备LAN或本批真实GGUF端到端。没有GitHub Actions Rust执行。当前检查点也不是新的二进制交付证明。
