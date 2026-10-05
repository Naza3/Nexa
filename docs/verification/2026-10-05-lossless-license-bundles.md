# 2026-10-05 许可整合源码与旧包语料验证

任务：L01，按[ADR0026](../decisions/0026-lossless-license-bundles.md)减少完整桌面许可文件至多10份，并无损保留原文/归属/对应源码。基线 `codex/dev` 的 `a14eb6fdb1858baf507c8b9a8509b0ca30df1316`；本任务没有提交或推送。整体桌面UI重设计是并行进行中的另一任务，不据此报告整批完成。

## 修改范围

- Python三个打包器：原许可库存无损汇集、直接可读HTML、原格式Microsoft DOCX/PDF兼容、完整桌面10文件硬门槛
- `download-engine/src/identity.rs`：新下载索引/11原文/框架/hash/归属及原build.files绑定，保留源码、PE、文件闭包与Windows句柄保护
- `xtask/src/package_acceptance.rs`：需要时验证bundle里的完整root notice，保持独立runtime自包含，不简单删必需项
- 合成Python/Rust共享fixture、变异/拒绝单测、只读旧ZIP语料检查脚本、打包说明/架构/决策/状态
- aria2源码锁、补丁、原构建脚本、原build-manifest及对应源码归档均未更改

## 已执行

均在Linux执行，退出码0：

1. `python3 -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'`：212项，208通过、4项既有Windows平台跳过。包含两种最终布局、原文/原索引字节、UTF-8/CRLF、重复许可归属、DOCX/PDF/UTF-16原件、tamper及重新计算外层hash后的合同拒绝。测试输出中的模拟诊断failed是既有负例，不是测试失败
2. Rust1.98.1 `rustfmt --edition 2024` 及 `--check`：格式检查通过；Rust执行见下文最新授权后的针对性回归
3. `python3 -X warn_default_encoding -W error::EncodingWarning -m py_compile`：本任务修改的Python模块；`git diff --check`：变更空白检查
4. `scripts/verify_license_corpus.py`：两份已交付旧ZIP各用固定SHA256读入；仅在临时目录改变许可展示和重算临时清单，调用生产Python校验器；退出后重新核对原ZIP字节hash。没有创建输出ZIP或执行EXE

## 原生a14语料

- 输入：`Nexa-Windows-x64-a14eb6f.zip`，17,316,051字节
- SHA256：`f01da636f5a2b87ae89c70f7b2f9ebf94d350931343f6cc275fe75e601c14492`
- 命令：`python3 -X warn_default_encoding -W error::EncodingWarning scripts/verify_license_corpus.py --archive <上述旧ZIP> --expected-sha256 f01da636f5a2b87ae89c70f7b2f9ebf94d350931343f6cc275fe75e601c14492`
- 766原文件中的748份许可/notice/index全量逐字节可恢复。桌面548、runtime187、download11份原索引/许可，另有两份独立root notice
- 临时完整桌面28文件，许可相关文件恰好10：桌面4、runtime4、下载2
- 12个非许可payload原字节完全不变；6个manifest/SHA256SUMS仅在临时fixture中按新布局重算
- 对应源码SHA256 `940db370d442ee1b433d35a543a15e8ed3d3eeb04e20ae4a0f3c5e1ebaa2cf2f`，原build-manifest SHA256 `16e9fafcf9455d7e211dffffa650fff42c267b16c995305525f9eb41cffd77b8`，两者均未变
- 机器结果：[native语料](2026-10-05-license-native-corpus.json)

## 旧交叉1845语料

- 输入：`Nexa-Windows-x64-1845f93-cross-test.zip`，SHA256 `4191ec7248a1413fed52d6ae03c43f31fd515a73271d47807ac42688f2428b40`
- 命令同上，改用该输入及其固定SHA；只处理 `Nexa-Windows-cross-test/desktop-windows/`，独立验收工具/外层说明不冒充桌面文件
- 桌面子目录778原文件中的760份许可/notice/index全量逐字节可恢复；桌面555、runtime194（两者均含root notice）、download11
- 两层各自的Microsoft原DOCX原样保留并可独立打开，root notice原字节收入各自文本；没有把DOCX转文本或存入新压缩包
- 临时完整桌面同样28文件、许可相关文件10（4+4+2）。12个非许可payload、原build-manifest及aria2对应源码均未变
- 对应源码SHA256 `d9a551e0fa366aafeb25816e62171b25c2619536b83f41e8d206e95e09e4c471`，原build-manifest SHA256 `b1fd304429dd3d0abc51c19bf27d342455b387094f66be9304788a6c598a35b9`
- 机器结果：[cross语料](2026-10-05-license-cross-corpus.json)

两份语料都保留完整原JSON库存字节，包括Cargo/npm以及交叉Microsoft索引；原记录的组件、版本、来源、hash/包integrity等仍能通过新索引找回。计数包含桌面全部嵌套文件和这些新索引，不计算未展开的aria2必要对应源码归档内部文件；该归档本身始终原样随包。

## 独立审查修复与实际Rust回归

独立审查指出验收器替代root notice分支只精确检查notice自身的归属，其他document可以被改成`[null]`或篡改组件/来源后重算外层hash而未被原逻辑拒绝。已将Rust与Python的原库存归属重建对齐：验证Cargo/npm/Microsoft原库存的完整记录、路径与hash、记录顺序和字段；拒绝归属遗漏/新增/篡改、空/null记录、重复键和无映射原文。同步收紧原路径、直接可读的HTML/Microsoft原格式存储和目录闭包。现有hash、PE、对应源码、Windows句柄和文件闭包保持。

最终独立审查还复现了数值精度缺口：原库存`18446744073709551616`与外层归属`18446744073709551617`可被serde JSON浮点回退舍入为同一值。两端现统一拒绝浮点/指数、超i64/u64范围整数及`-0`；嵌套字段同规则，合法i64/u64边界实测保留。Python归属比较还严格区分bool/int，拒绝`true`与`1`、`false`与`0`混淆。新增原库存/外层归属及类型反例均通过；修复后重跑两份旧ZIP语料，748/760份原文和10文件结果与先前机器JSON完全一致。

用户最新明确要求“这批修改完成后github构建”，据此恢复本批提交前必要Rust回归；此前未执行Rust的状态已被以下新证据更新。使用现有工具链和已有native目录，没有重建原生库：

- 环境：`source /workspace/shared/nexa-cloud-tools/env.sh`，`RUSTUP_TOOLCHAIN=1.98.1-x86_64-unknown-linux-gnu CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 CARGO_NET_OFFLINE=true AIR_NATIVE_DIR=$PWD/build/onboarding-linux/native`，取消`AIR_NATIVE_PROFILE`
- `cargo test --locked -p download-engine -p xtask --all-targets`：退出码0；download-engine28通过，独立nexa-acceptance25通过/1既有真实产品+固定GGUF用例忽略，xtask24通过，合计77通过/0失败/1忽略
- `cargo clippy --locked -p download-engine -p xtask --all-targets -- -D warnings`：退出码0
- 两份Python/Rust共享fixture均由原生消费者实际验证；包含重新封闭manifest/SHA256SUMS后的逐document归属变异、原库存hash/路径/记录闭包、重复JSON键及格式/offset/hash/原件存储反例
- 严格Python新增覆盖每份runtime原文的所有归属字段变异，总计212项（208通过/4平台skip）

## 未执行与下一步

本轮尚未运行完整Rust workspace、原生Windows编译/测试、真实推理、新ZIP生成或GitHub Actions，没有提交或推送。这里的Linux针对性Rust通过不等于完整workspace或Windows通过；两份旧包语料也不等于新版二进制交付。

最后实际交付仍是a14原包，[Windows Actions37216837409](https://github.com/Naza3/Nexa/actions/runs/37216837409)已在2026-10-04成功；新布局没有新CI或实机结论。桌面整体UI精简仍由并行任务继续，待本批修改和联合验证完成后按用户最新授权统一提交并在GitHub构建。原生Windows组件/包、下载与独立验收器真实执行，以及目标Win10窗口、两机LAN、离线/长期测试继续分别记证据。
