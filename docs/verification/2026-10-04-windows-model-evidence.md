# 2026-10-04 Windows本机测试记录与反馈

基于长期开发分支 `codex/nexa-add-model` 的整合提交 `f496aac6da0f28980ceee15211ff4a8eddf0ff26`。按ADR0022修复Windows canonical数据目录误被外部路径策略拒绝、证明读写错误被隐藏及界面结果与历史矩阵混用。未修改main或固定vendor。

## 实施

内部数据目录原样保留PathBuf，仅额外接受本地VerbatimDisk；外部来源、picker、扫描、下载仍保持严格策略。UNC/设备/通用Verbatim和reparse仍拒绝，逐级句柄/身份保护保持。库存、记录与实例观察一起修正。

记录schema1不迁移；损坏与悬空记录不作为缺失默认重置，文件保留且模型仍展示。scope、engine双文件身份、读/写失败使用有界安全错误码。真实生成通过还需当前精确scope和成功记录，不能让旧Passed掩盖本次结果。

前端本次操作与持久记录分别显示；模型行有spinner、耗时、毫秒完成时间和原因。当前驻留显示已加载，合法候选显示加载模型，历史validated不作为开关。raw passed与回读stale/缺失/故障、忙碌/未ready、晚回/关闭及极大timestamp均有回归。

## 实际源码验证

- Rust全workspace/all-targets：40组449通过、0失败、7既有忽略；另一次默认workspace含4项通过的doc-tests，共453通过
- 全workspace/all-targets clippy `-D warnings`、fmt与diff检查exit0
- 前端typecheck、lint、18文件343项测试、production build exit0（原291加52，不与子集累加）
- Python scripts155项：153通过、2Windows平台skip。首次发现harness新增阶段/布尔字段未同步Python封闭消费协议；同步后全量通过，未知键与非bool仍拒绝
- Windows MSVC target下model-store/runtime-api/runtime-cli/desktop-bridge all-targets交叉检查exit0，包含Windows gated canonical/尾点空格、替换与离线记录测试的编译
- 作者Rust四crate258/0/1、独立Rust72、独立前端87均属于上述回归子集，不相加

harness增加真实模型重复短测、唯一记录更新，以及Windows外部零复制模型的加载测试→重复→canonical离线记录断言；Python验收强制相应新字段，不接受旧报告代替。

## 验证边界

未执行Windows原生测试、真实窗口、外部文件共享锁或两设备LAN。Windows gated用例仅交叉编译，不宣称实机通过。本检查点未运行新版Linux真实GGUF端到端，也没有新二进制交付；这些随超时/不卸载设置完成后统一验证构建。没有GitHub Actions Rust任务。
