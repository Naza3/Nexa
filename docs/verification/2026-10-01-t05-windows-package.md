# T05 Windows x64 CPU 便携包验证

日期：2026-10-01。状态：实现进行中，Windows Release 包/目标机尚待实际证据。T04 已验收源为 `ccb2053fe514f582f6161f9fc87ee25346aa55e4`，文档基线为 `15c5d33d5005de8563f074a342291ac6331d1c73`。本记录不把已有 T04 CI 当成 T05 结果。

## 范围与固定输入

实现边界见 [ADR 0006](../decisions/0006-t05-windows-portable-package.md)。沿用 Rust1.98.1、CMake4.4.3、llama `2149c00f4442dc59302e134a02e4c99d5f7ed9fc` 与既有 Windows job 的 Release 原生树；产品/工具使用真正 Rust Release。模型 Qwen3-0.6B Q8_0 的 SHA-256 为 `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`，模板为 `57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361`。真实短验参数 CPU/context2048/threads2/batch128/gpu_layers0，不更改 generic4096。

## 分层结果

| 验证层 | 状态 | 证据/界限 |
| --- | --- | --- |
| T00–T04 已有固定 Windows 基线 | 已通过既有阶段 | [T04 记录](2026-10-01-t04-http-cli.md)，CI36816604494；不是本轮 Release 包 |
| 包/验收代码与 Linux 逻辑回归 | 最终开发聚合通过 | Rust236pass/6ignored、native identity3pass；Python父级30pass后边界修补全32pass，真实模型另行分层记录 |
| CI 证据 staging 安全回归 | 通过局部验证 | 下节实际命令与结果；不证明 Windows 产品可运行 |
| Windows Release 构建、PE/许可/manifest/hash/ZIP | 未执行 | 等待精确源提交的 Actions；工作流已写不等于已运行 |
| 中文空格新解压目录、空 CWD、受限 PATH 真实 CLI/API | 未执行 | 必须跑解压产品与解压独立工具，不能用 debug/仓库二进制替代 |
| Windows 10 i5-8400 /16GB 本地短验 | 未执行 | 需本地实际 OS build 与包/工具/report identity |
| 无开发工具独立机器、VC预装状态、实际离线 | 未验证 | PATH 清理、Server2022 或机器角色声明不足以证明 |
| Windows 11 / Android /长期内存与性能 | 未执行 | 不在本轮短包 CI 结论内 |

## 已执行的证据工具验证

2026-10-01，本地 Linux 开发环境：

- `python -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p test_stage_ci_evidence.py -v`：exit0，9项通过。包括闭合文件允许列表、模型/凭据/data/PDB/未知log不上传、伪装PE拒绝、重复JSON键与symlink拒绝、凭据/正文拒绝及失败报告保留、原失败/参数/hash链保留、T05失败report和CI step outcome不误改
- `python -X warn_default_encoding -W error::EncodingWarning scripts/stage_ci_evidence.py --source artifacts/verification/t04-windows-36816604494 --out artifacts/verification/t05-staging-compatibility`：exit0；25份已审查报告/日志保留，7份合成stdout/stderr仅留hash与字节数，无拒绝项。原始下载目录未改
- 兼容性复查：`upstream-processes.json` 的 baseline_result=pass、diagnostic_result=failed 保持；HTTP98项检查保留原结果。staging 只改路径表示和文件编码，source_sha256 与输出sha256分列，不把脱敏文件hash冒充原始artifact hash

实际留存 `artifacts/verification/t05-staging-compatibility/evidence-index.json`；这是局部工具回归，不是 T05 Windows CI 产物。

## Windows CI 执行契约（结果待填）

原有 T00–T04 所有检查保留。新增：

1. 独立 `rustc --edition 2024 --test crates/llama-adapter/native_identity.rs` 检查原生身份边界；测试 EXE 放 build，不混入产品或报告
2. `cargo run --locked -p xtask -- build --platform windows-x64 --backend cpu`；只增量复用 `build/native-release`，明确 Rust Release target；模型不打包
3. 校验 `dist/{windows-x64-cpu,acceptance-tools}.zip.sha256`，解压至新的中文/空格目录；将既有公开fixture复制到自有`模型 输入/候选 模型.gguf`并核对副本hash相同，工具实际导入/原生load临时根也须含中文/空格。在非 repo 空 CWD 运行 `nexa-acceptance.exe --package <解压产品目录> --model <固定外部GGUF> --out <包外报告> --machine-role ci --disconnect-cycles 50`
4. 要求两个产品/工具manifest在打包目录与解压目录均project_commit=GITHUB_SHA、project_dirty=false，解压manifest与原始hash一致，全部SHA256SUMS与实际文件数一致；验收exit0、short_package_checks_passed=true、product.project_commit=GITHUB_SHA、machine_role_user_declared=ci、HTTP断流requested/attempted/passed均50，产品/模型/CWD/导入模型data目录的路径观测均有非ASCII及空格；A20 必须仍明确 unverified
5. 精确上传产品ZIP/hash、独立工具ZIP/hash、存在的独立PDB和脱敏允许列表证据；失败也保留安全报告。无 GitHub Release

待获得实际 run 后补：精确 commit/tree/run/job、OS build/CPU/工具链、测试数量/exit、native复用证据、产品/工具ZIP大小及SHA、manifest/hash、PE闭包、许可清单、真实短验结果与所有skip/unverified。未经实际日志不得填入通过或零失败。

## 本地验收与结束条件

用户已表示包构建好后可在本地验收，未授权远控。取得同一源提交的两个ZIP与各自hash，解压为相邻 `windows-x64-cpu` / `acceptance-tools`；只需执行独立验收器并提供现有固定模型和报告路径，不需 Cargo/Python/VS。默认产品目录可自动定位，移到别处时显式 `--package`。工具只创建自有临时data/测试token，并在结束时关停清理；不初始化长期真实凭据。

系统实际 build 由工具观测；开发工具/VC预装/离线条件需实际观察或明确用户声明分别记录，不能根据 PATH 或OS名称推断。短检查过后T05保持待验证，直到独立机器/目标设备门槛有对应真实证据；T06不提前开始。异常不要求修改安全设置或从任意DLL站补文件。

## Linux 实现层回归（不是 Windows 包结论）

打包实现工位的实际证据已核读：`artifacts/verification/t05-linux-build/workspace-tests.log`合计233 passed / 0 failed / 6 ignored，包含当时的验收器逻辑测试；随后新增参数/counter/path观测的最终聚合须重新记数。`clippy.log`与format检查exit0，独立`native-identity-tests.log`为3 passed / 0 failed。脚本在Linux运行的`non-windows-refusal.log`真实exit1，明确拒绝把Linux称Windows构建，无Windows包生成。环境容量/陈旧测试二进制排错不作为Windows产品通过证据。

2026-10-01，CI整合工位再执行`python -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py' -v`，exit0，28项通过（上游runner6、package13、evidence9）。日志`artifacts/verification/t05-ci-integration-python.log`。YAML可解析为23个步骤、所有ID唯一、既有upstream及HTTP50仍在；文档链接/围栏和`git diff --check`通过。这些是静态/开发验证，未运行PowerShell或Windows。

独立验收器的真实开发生命周期初轮修正：在线导入响应是嵌套`model.validated`，验收器原断言错误，修正只改测试工具，不改产品响应。首轮失败报告`artifacts/verification/t05-package-lifecycle-linux-attempt1.json`保留。修正后命令：

```sh
NEXA_ACCEPTANCE_CLI="$CARGO_TARGET_DIR/debug/ai-runtime" \
NEXA_ACCEPTANCE_MODEL="$PWD/models/Qwen3-0.6B-Q8_0.gguf" \
NEXA_ACCEPTANCE_REPORT="$PWD/artifacts/verification/t05-package-lifecycle-linux.json" \
cargo test --locked --offline -p xtask --bin nexa-acceptance \
  package_acceptance::tests::real_product_lifecycle_uses_shared_oracle_and_reaps \
  -- --ignored --exact --nocapture
```

实际exit0、1项独立真实测试通过，用时50.31秒。已核读JSON：13项生命周期pass，HTTP44pass/0fail/9skipped，5次断流恢复及所有actual CLI检查通过；短命服务退出、标记消失、实例锁释放与自有临时data清理确认。固定模型/参数与上文一致。该报告SHA-256 `d59d99b32d9d59904aa76302f62a590c57b0eacda9409502dd6ea0557a0872ad`。

报告故意保留`product=null`、`short_package_checks_passed=false`、A20 unverified，因为这是Linux实现生命周期，并未验证Windows PE包。`--machine-role ci`只说明该测试所声明角色；实际os=linux。后续CI新增Release50次断流，不能将此旧5次报告追溯改成50。此报告已通过证据stage字段安全门，不包含正文或凭据。


### 最新显式50次开发生命周期复验

2026-10-01，验收器参数/counter与路径观测扩展后的独立检查：19项helper、24项xtask主binary测试通过，fmt、strict clippy及MSVC all-targets交叉check均exit0。交叉check不是Windows实际执行，最终全workspace数量见下节父级收口记录。

上节同一真实命令增加`NEXA_ACCEPTANCE_DISCONNECT_CYCLES=50`，报告改为`artifacts/verification/t05-package-lifecycle-linux-50.json`，实际exit0、1项真实测试通过、61.38秒。报告已重新核读：13项生命周期pass、HTTP89pass/0fail/9skipped；`disconnect_cycles_requested/attempted/passed=50/50/50`。SHA-256为`48768509a9fd31959ae3954b299b3dbad8321f8bd3175d9c50bed814695499e3`。该新报告也通过证据stage字段安全门。

`path_coverage`实际观察临时CWD及受控模型data目录均有非ASCII/空格；本次Linux产品EXE与原模型源路径仍为ASCII且无空格，未把它们标成已覆盖。Windows CI配置进一步复制/解压以覆盖这两项并逐项断言，等待实际执行。报告仍`product=null`、短Windows包结论false、A20 unverified，不能把50次Linux开发链路当Windows发行成功；旧5次与最初失败报告完整保留。


## 发布前最终开发聚合与证据读取上限修补

2026-10-01 05:36 UTC，父级最终聚合日志`artifacts/verification/t05-final/{format,clippy,workspace,windows-check,python,native-identity}.log`已核读：

- format、workspace all-targets strict clippy、MSVC all-targets交叉check均exit0
- workspace **236 passed / 0 failed / 6 ignored**，按每个测试/文档测试结果重新求和；6项真实模型测试在普通聚合中显式ignored，既有5项真实回归和新增验收生命周期均有独立证据，不称本次普通suite执行了它们
- 当时Python全套 **30 passed**；native identity独立 **3 passed / 0 failed**

随后只修`scripts/stage_ci_evidence.py::read_regular`的明确内存上限漏洞：先stat拒绝大于16MiB，再只读`16MiB+1`以识别stat后并发增长；读取长度超限立即拒绝，不先无界`read_bytes`再检查。不改变允许列表、正常报告内容或Rust源码。

新增两项确定性回归：真实稀疏超大文件必须在open前拒绝；通过受控钩子在stat后增长同一文件，用缩小测试上限证明只请求/收到`limit+1`字节。`python -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p test_stage_ci_evidence.py -v`实际exit0，**11 passed**，日志`artifacts/verification/t05-final/evidence-bounded-read.log`。随后全Python快速复验实际exit0，**32 passed**，日志`artifacts/verification/t05-final/python-after-evidence-bound.log`。文档链接/围栏、YAML解析与`git diff --check`再次通过。

本节不改变Windows实际Release包/目标设备仍待验证的结论，也不覆盖上述原始失败或早期计数；没有重新运行Rust构建或改动Rust源。

## 最终真实回归补充

2026-10-01 05:40 UTC，在同一冻结工作树上显式执行 `cargo test --locked --workspace -- --ignored --test-threads=1`，六项真实模型测试全部通过，exit0；固定模型与推理线程仍为本记录的Qwen3 SHA及2线程。日志为 `artifacts/verification/t05-final/real-workspace.log`。其中独立验收器开发链再次通过13项生命周期检查、HTTP 89 pass / 0 fail / 9 skipped，断流 requested / attempted / passed = 50 / 50 / 50；报告 `artifacts/verification/t05-final/real-package-lifecycle.json` 的SHA-256为 `92a43c3336e1c9db8d23a7f6094335878f3688b19dfb4ec0cb1efff4f7c249bc`。该结果仍是Linux开发链路，不能替代Windows Release包或Win10目标机验收。
