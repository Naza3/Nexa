# T05 Windows x64 CPU 便携包验证

日期：2026-10-01。状态：T05按用户最新缩定范围已完成；源码6a7e9d0的Windows Server2022 Release包及独立验收已通过，用户Windows10 build19044 / i5-8400短验报告已回且通过；A20无开发工具/离线与长期稳定性经用户批准移至后期验证，仍无通过证据，T06开始。T04 已验收源为 `ccb2053fe514f582f6161f9fc87ee25346aa55e4`，文档基线为 `15c5d33d5005de8563f074a342291ac6331d1c73`。本记录不把已有 T04 CI 当成 T05 结果。

## 范围与固定输入

实现边界见 [ADR 0006](../decisions/0006-t05-windows-portable-package.md)。沿用 Rust1.98.1、CMake4.4.3、llama `2149c00f4442dc59302e134a02e4c99d5f7ed9fc` 与既有 Windows job 的 Release 原生树；产品/工具使用真正 Rust Release。模型 Qwen3-0.6B Q8_0 的 SHA-256 为 `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`，模板为 `57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361`。真实短验参数 CPU/context2048/threads2/batch128/gpu_layers0，不更改 generic4096。

## 分层结果

| 验证层 | 状态 | 证据/界限 |
| --- | --- | --- |
| T00–T04 已有固定 Windows 基线 | 已通过既有阶段 | [T04 记录](2026-10-01-t04-http-cli.md)，CI36816604494；不是本轮 Release 包 |
| 包/验收代码与 Linux 逻辑回归 | 最终开发聚合通过 | Rust236pass/6ignored、native identity3pass；Python父级30pass后边界修补全32pass，真实模型另行分层记录 |
| CI 证据 staging 安全回归 | 通过局部验证 | 下节实际命令与结果；不证明 Windows 产品可运行 |
| Windows Release 构建、PE/许可/manifest/hash/ZIP | 固定CI通过 | 6a7e9d0 / run36829233039；产品/工具ZIP已下载并独立逐文件/hash/PE复核，前四轮失败保留 |
| 中文空格新解压目录、空 CWD、受限 PATH 真实 CLI/API | 固定CI通过 | 解压Release产品16项包检查通过、HTTP89pass/9skip、断流50/50/50，实际回收/清理与包未修改通过 |
| Windows 10 i5-8400 本地短验 | 已通过短验 | workstation build19044 / 6逻辑CPU；16包pass/2skip、HTTP44pass/9skip、断流5/5/5；identity匹配交付包，RAM未核实 |
| 无开发工具独立机器、VC预装状态、实际离线 | 未验证 | 用户声明安装过大部分开发工具且测试联网；VC预装未知，PATH清理不证明A20 |
| Windows 11 /长期内存与性能 | 未执行 | 不在本轮短包 CI 结论内 |

## 已执行的证据工具验证

2026-10-01，本地 Linux 开发环境：

- `python -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p test_stage_ci_evidence.py -v`：exit0，9项通过。包括闭合文件允许列表、模型/凭据/data/PDB/未知log不上传、伪装PE拒绝、重复JSON键与symlink拒绝、凭据/正文拒绝及失败报告保留、原失败/参数/hash链保留、T05失败report和CI step outcome不误改
- `python -X warn_default_encoding -W error::EncodingWarning scripts/stage_ci_evidence.py --source artifacts/verification/t04-windows-36816604494 --out artifacts/verification/t05-staging-compatibility`：exit0；25份已审查报告/日志保留，7份合成stdout/stderr仅留hash与字节数，无拒绝项。原始下载目录未改
- 兼容性复查：`upstream-processes.json` 的 baseline_result=pass、diagnostic_result=failed 保持；HTTP98项检查保留原结果。staging 只改路径表示和文件编码，source_sha256 与输出sha256分列，不把脱敏文件hash冒充原始artifact hash

实际留存 `artifacts/verification/t05-staging-compatibility/evidence-index.json`；这是局部工具回归，不是 T05 Windows CI 产物。

## Windows CI 执行契约（第五轮实际结果见末节）

原有 T00–T04 所有检查保留。新增：

1. 独立 `rustc --edition 2024 --test crates/llama-adapter/native_identity.rs` 检查原生身份边界；测试 EXE 放 build，不混入产品或报告
2. `cargo run --locked -p xtask -- build --platform windows-x64 --backend cpu`；只增量复用 `build/native-release`，明确 Rust Release target；模型不打包
3. 校验 `dist/{windows-x64-cpu,acceptance-tools}.zip.sha256`，解压至新的中文/空格目录；将既有公开fixture复制到自有`模型 输入/候选 模型.gguf`并核对副本hash相同，工具实际导入/原生load临时根也须含中文/空格。在非 repo 空 CWD 运行 `nexa-acceptance.exe --package <解压产品目录> --model <固定外部GGUF> --out <包外报告> --machine-role ci --disconnect-cycles 50`
4. 要求两个产品/工具manifest在打包目录与解压目录均project_commit=GITHUB_SHA、project_dirty=false，解压manifest与原始hash一致，全部SHA256SUMS与实际文件数一致；验收exit0、short_package_checks_passed=true、product.project_commit=GITHUB_SHA、machine_role_user_declared=ci、HTTP断流requested/attempted/passed均50，产品/模型/CWD/导入模型data目录的路径观测均有非ASCII及空格；A20 必须仍明确 unverified
5. 精确上传产品ZIP/hash、独立工具ZIP/hash、存在的独立PDB和脱敏允许列表证据；失败也保留安全报告。无 GitHub Release

下面保留每轮原始结果，第五轮记录已补齐精确commit/tree/run/job、OS/CPU/工具链、实际测试与产品身份/闭包/许可/报告。用户Windows10短验已回，另见末节；A20无开发工具/离线继续保持未验证。

## 本地验收与结束条件

用户已表示包构建好后可在本地验收，未授权远控。取得同一源提交的两个ZIP与各自hash，解压为相邻 `windows-x64-cpu` / `acceptance-tools`；只需执行独立验收器并提供现有固定模型和报告路径，不需 Cargo/Python/VS。默认产品目录可自动定位，移到别处时显式 `--package`。工具只创建自有临时data/测试token，并在结束时关停清理；不初始化长期真实凭据。

系统实际 build 由工具观测；开发工具/VC预装/离线条件需实际观察或明确用户声明分别记录，不能根据 PATH 或OS名称推断。原门槛要求独立无开发工具/离线证据；2026-10-01用户在目标机短验通过后明确批准将这两项和长期稳定性延后，当前T05据缩定范围收口并继续T06，未验证项保留不改写。异常不要求修改安全设置或从任意DLL站补文件。

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


## 首轮 Windows CI 与加载取消测试等待修复

实现提交 `f07cb337cb5e118c8cc2dda0dae602846d384b65` 的 [run 36821525300](https://github.com/Naza3/Nexa/actions/runs/36821525300) 于2026-10-01 05:57:54 UTC失败。job `110237927560` 第7步Rust测试在 `secure_transport_contract::fin_and_rst_cancel_load_prepare_stream_and_nonstream_without_stranded_pumps` 读取`cancelled=false`，该测试binary为9 pass / 1 fail。原生构建、CTest、native identity边界已通过；模型与Release打包/验收未执行，没有可交付产品包。原始日志没有FIN/RST子场景，不能事后补造具体case。

确定性fixture证实旧等待条件存在窗口：core处理取消时先移除active job，Load executor仍待完成；此时Loading + active=None + 无generation账本会使旧`clean()`提前返回。保持连接打开时active实际为Some，初次错误前置假设已通过失败日志保留并纠正。新的fake取消完成gate固定了真实窗口，旧oracle红测exit101；只修改测试文件，断连专用等待保留原3秒总deadline，联合检查取消确认、active/queue为空、输出预算为0及离开Loading/Generating/Unloading。普通成功路径clean未变，生产HTTP/core/worker均未修改。gate通过RAII在panic/timeout时释放。

受控回归红转绿；API 24+4+11=39项通过；20轮定向重复通过，包含160个FIN/RST子场景和20次gate回归。strict clippy、MSVC all-targets check、fmt及diff检查均exit0。修后诊断仅记录rst/mode/stream/phase与状态数值。证据为`artifacts/verification/t05-disconnect-*.log`，包括原始fixture编译/前置断言失败、oracle-red/oracle-green/api-linux/focused-repeat-linux/clippy-linux/msvc-check/format；此处不是Windows重跑通过证明。

首轮证据ZIP SHA-256为`2041e3d8196b14285711b511be0200f697cd9042eb16878520ca78c9b2909d2e`（17,457 bytes）；failure-only原生工具ZIP SHA-256为`7b127b305333adcfb0b1a024601cb58f3708da8d3932cec6fdedee91852d81a6`（29,211,224 bytes），14项工具manifest大小/hash均核对。原件保留于`artifacts/verification/t05-windows-36821525300*`，不删除或把原失败改成通过。

## 第二轮 Windows CI：前置真实回归通过，打包环境初始化失败

取消oracle定向修复提交 `732bf75e3f30ab2b549a6569fa8ebda791685629`（tree `b4ff2d45ecaaeb3e3543b0e39c70edb2fb8a9730`）的 [run 36823563860](https://github.com/Naza3/Nexa/actions/runs/36823563860)，attempt1 / job `110244165275`，06:12:36 UTC启动，06:30:31 UTC终态failure。只读取证后保持原失败，没有在该run盲重跑或跳过检查。

### 实际通过的前置检查

- Windows Server2022 x64，实际OS build `10.0.20348.5622`，runner image `20260920.314.1`；不是Windows10目标机
- 缺native目录的管理harness/CLI/xtask独立构建、VS2022原生Release/CTest、native identity3项通过；Windows junction路径边界确有实际执行
- Rustfmt、workspace测试与all-targets strict clippy通过。按`windows-rust-tests.log`逐测试binary/文档测试求和：**234 passed / 0 failed / 6 ignored**；取消FIN/RST原失败及新增gate回归在本轮真实Windows已通过。Windows条件编译数量不能直接替换Linux计数
- Python **32 passed**；固定模型/模板身份、上游生成/五次bench、native真实suite、既有5项独立真实模型回归、独立管理/worker真实链路全部通过。第6项ignored为独立验收器开发生命周期，本轮未把它在普通suite中执行
- T04真实HTTP/CLI仍用debug产品，**89 pass / 0 fail / 9 skipped**，断流requested/attempted/passed=**50/50/50**；wrapper56.657秒，initialize/online_import/api_smoke/service_exit均0，forced_cleanup=false
- 上游2线程必需baseline=pass；4线程超配诊断仍failed，原状态保留，不因CI前置绿色改称所有配置通过

### 第16步明确失败

`cargo run --locked -p xtask -- build --platform windows-x64 --backend cpu`在06:30:16.361启动，06:30:16.502报告：

```text
Windows package failed: command failed (1): cmd.exe
The network path was not found.
```

失败处为`scripts/package_windows.py::selected_visual_studio`调用既有`VsDevCmd.bat`取开发环境，早于Rust Release产品构建。源码通过Python argv列表向`cmd.exe /d /s /c`传递已经嵌套引号的整段batch命令；`subprocess.list2cmdline`会再做CRT式反斜杠引号转义，而cmd解析语法不同，这是限定排查/修复范围。该日志没有记录最终packager commandline或所选dev路径，不能把推测参数冒充实际输出，也不能仅凭错误文本宣称远程网络失效。

已把精确失败与完整日志交由打包工位修复引号/调用契约并补回归；不得削弱VS/CRT来源校验或绕过初始化。产品/独立工具ZIP、PE闭包/CRT签名与中文空格解压Release验收均未执行或未生成，不能用此前debug真实HTTP50代替。T05仍进行中，A20/T06门槛未完成。

### 下载证据完整性

- 脱敏证据artifact `11145211337`：ZIP37,847 bytes，SHA-256 `9f9638bec24ec36f232e8f282caac2102a28f9c9e62ff63fb9def549bca1e5eb`；下载后与GitHub digest一致，26份报告的stage输出hash逐项匹配，index明确portable_package=failure/package_acceptance=skipped
- failure-only原生诊断artifact `11145106358`：ZIP29,186,727 bytes，SHA-256 `917a94880fad9a8bb3ea6b4e4b460d8b0ffd5d2602df8a4caa4ab3242512d02e`；下载后核对archive、精确source commit及各项工具manifest大小/hash。这是诊断工具，不是产品包
- 原件/解包/完整job日志/run及artifact元数据位于`artifacts/verification/t05-windows-36823563860*`。下载服务首次503后受控重取成功，与CI本身故障分开记录；未删改前次run36821525300的失败证据

### VsDevCmd 窄修与发布前回归

本次修复只涉及打包脚本调用边界及专属测试，不改变VS选择、x64/Release、CRT来源、许可、PE闭包或用户安全设置。`VsDevCmd.bat`使用完整原始CreateProcess命令行，明确可执行文件为`SystemRoot/System32/cmd.exe`，`shell=False`，不再让Python对cmd程序文本套用CRT argv转义；显式`/d /s /u /v:off`，通过UTF-16LE解码`set`输出保留中文环境值，并关闭延迟扩展。batch失败仍必须返回失败，不能退回继承的Redist环境伪装初始化成功。

跨平台回归验证精确调用参数/引号规则与错误传播；新增真实Windows用例以受控临时batch模拟开发环境，路径包含中文、空格、括号、`&`与`!`，检查三个参数、PATH/Redist取值和exit/b7失败传播。该用例在非Windows明确skip；在原有CI的`Install fixed development tools`步骤由`unittest discover -s scripts -p 'test_*.py'`自动执行，早于原生编译和第16步打包，不用等完整CI结束才发现cmd解析错误。

父级实际运行Python全套：**35项收集，34 passed / 1 Windows专用 skipped**，exit0；`git diff --check`通过。这只证明当前主机的回归，真实Windows cmd用例和最终Release包仍等待新提交的CI。没有将skip称为通过，也没有改写run36823563860的原始失败。

## 第三轮 Windows CI：真实cmd回归暴露fixture参数假设

窄修提交 `f044335fa723f06c3fede94efa49018080a6479f`（tree `4b4aeede9982079de2c7403a1e011199e7a6e152`）的 [run 36825830693](https://github.com/Naza3/Nexa/actions/runs/36825830693)，attempt1 / job `110251188861`，06:38:14 UTC启动，06:39:38 UTC终态failure。第3步工具安装成功后的Python前置测试收集35项，**34 passed / 1 error**；错误来自新增的真实Windows用例，确实执行而非skip。

`test_real_windows_devcmd_with_spaces_unicode_and_failure`在`test_windows_package.py:69`调用`devcmd_environment`，由`package_windows.py:242`报告`Visual Studio environment initialization failed (22): cmd.exe`，stdout为空。该fixture明确将exit22用于`%2`不等于`-arch=x64`；其前一项`%1`检查的exit21未触发，说明batch已经实际启动并运行到第二参数断言，不能把此错误等同第二轮无法启动的network-path错误。

cmd batch会对未引用的等号参数进行自己的分隔，fixture对`%2`的期待成为限定排查范围；本轮日志没有打印实际`%2/%3`值，不倒填它们。生产标准VS参数、来源/运行库检查保持不变，由打包工位核实并修正测试oracle。真实Windows用例在昂贵编译之前失败，实现了前置护栏；本轮native、Rust、模型、Release包及解压验收全部skipped，不能继承上一run的成功为本轮通过，也不能称已有可交付ZIP。

脱敏artifact `11145371348`，ZIP2,551 bytes，SHA-256 `9341fdb3c27358f6db4431a2f1b1db601de20a44ebb37c662d90aa847112ecae`。下载后与GitHub digest相同、stage各项输出hash核对通过；完整日志、run/jobs/artifact元数据、ZIP及解包保存在`artifacts/verification/t05-windows-36825830693*`。没有触发重复CI，原始三轮结果均保留；T05仍进行中，Windows10/A20未验证。

第三轮后的窄修仅改真实Windows fixture：记录原始`%*`和`%1`至`%5`到受控`NEXA_FIXTURE_*`环境字段，由Python分别精确断言完整三个标准flags以及等号分隔后的五个batch参数槽；删除fixture内部错误的三槽if-exit断言。生产打包脚本完全不变，中文/空格/括号/`&`/`!`路径、实际PATH/Redist取值和exit/b7负例均保留。Linux回归仍为35项收集、34 passed / 1 Windows专用 skipped；新五槽期望的实际Windows观测等待下一run，不把本次skip或预期值当成已测结果。

## 第四轮 Windows CI：真实cmd回归通过，签名cmdlet模块加载失败

fixture修复提交 `29dd01e75832852b426f6a31ef56c1cd2358033d`（tree `40f88b87e2b317bed970ffec7bca38ddbc4b6d90`）的 [run 36826425239](https://github.com/Naza3/Nexa/actions/runs/36826425239)，attempt1 / job `110253024795`，06:44:50 UTC启动，07:05:20 UTC终态failure。

### 已观察通过的范围

第3步Python **35 passed，无skip**，新增真实Windows cmd用例已实际通过：中文/空格/括号/`&`/`!`路径，标准flags原始文本与五个batch参数槽，PATH/Redist环境和exit/b7失败传播均得到该测试验证。这确认第三轮fixture窄修，不能替代最终产品验收。

管理端独立构建、原生Release/CTest、native identity3项、format、workspace strict clippy和Rust测试均通过；Rust实际合计 **234 passed / 0 failed / 6 ignored**。固定模型身份、上游/原生suite、既有5项独立真实模型及管理/worker链路均通过。T04 debug HTTP/CLI报告 **89 pass / 0 fail / 9 skipped**，断流requested/attempted/passed=**50/50/50**，wrapper57.469秒，所有生命周期步骤exit0、forced_cleanup=false。旧4线程超配诊断仍单列failed，不改变支持范围。

### 第16步实际失败及证据边界

打包于07:01:14.346开始，07:05:06.402返回`command failed (1): powershell.exe`。`Get-AuthenticodeSignature`发现`Microsoft.PowerShell.Security`模块，但模块无法加载；错误ID为`CouldNotAutoloadMatchingModule`。错误展示的脚本片段只包含`...Microsoft.VC143.CRT\vcruntime140.dll`尾段。它是签名检查工具加载失败，不能解释成DLL签名无效，也不能放宽`Valid`和Microsoft来源要求。

日志显示外层workflow使用`C:\Program Files\PowerShell\7\pwsh.EXE`，冻结打包脚本则通过裸`powershell.exe`启动子进程并继承开发环境。实际子PowerShell绝对路径、PSModulePath、完整Redist目录/版本及Import-Module底层异常均未记录；跨宿主模块搜索冲突只是排查候选，不作为已证明根因。不得借此修改ExecutionPolicy、Defender或系统配置。

脚本控制流在DLL签名检查前安排了三个Release构建命令，但它没有输出成功命令的逐步日志，本轮也未上传最终Release EXE、manifest或build-result。**不能单凭到达签名检查推定并对外宣称三份Release二进制已验收**。可实证的是签名cmdlet对`vcruntime140.dll`的调用失败。完整产品/工具ZIP尚未生成或上传，解压后真实Release/UTF8路径/50断流验收未执行；前置debug HTTP50不替代它们。

### 原始证据

- 脱敏artifact `11145848691`：ZIP37,910 bytes，SHA-256 `3df37804b291f337d423e923da035d487b221a98839c6cc0a69ef8d50f85502f`；已下载核对GitHub digest和26份stage报告hash
- failure-only原生诊断artifact `11145953184`：ZIP29,186,710 bytes，SHA-256 `744e5c0cea21116a887bfbb32cec75f3a1bfd336c93e5416f73e74e43a397006`；已下载核对archive、源commit及14项manifest文件大小/hash
- 原始job/run/jobs/artifact元数据、ZIP和解包保存于`artifacts/verification/t05-windows-36826425239*`；未重跑或删除旧失败。打包工位负责限定的子PowerShell环境诊断；本记录没有提前声明修复结果

T05保持进行中。Windows10目标机/A20/独立离线无开发工具条件仍未验证，T06门槛未打开。

### PowerShell 签名工具边界窄修（真实Windows待新CI）

本次仅固定签名工具的宿主与模块解析边界：使用`SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe`；复制子进程环境并仅移除继承的`PSMODULEPATH`，不修改父进程、用户或系统环境。显式按绝对系统路径加载Security、Utility、Management模块，禁止自动换用其他目录的同名模块；失败记录宿主/版本及实际模块异常类型、ID和消息。原有`Valid`且Microsoft签名者要求不变，没有安装软件、降低签名要求或修改ExecutionPolicy/Defender/网络设置。

新增前置真实Windows测试先在原继承环境显式导入模块，记录实际底层错误的脱敏诊断，不输出完整环境。随后用固定系统host验证系统PowerShell可执行文件的有效Microsoft签名，同时要求未签名临时ps1被拒绝；该ps1不会被执行。测试由现有早期Python发现步骤收集，仍先于完整native/Release编译。

父级实际执行Python全套：**38项收集，36 passed / 2 Windows专用 skipped**，exit0；diff检查通过。PS7模块搜索路径污染继续仅列为候选，待下一Windows实际诊断确认；不能将本次Linuxskip、新代码存在或旧日志到达签名检查解释为Windows签名验证/Release包已通过。

## 第五轮 Windows CI：固定Release包与独立验收通过

源码 `6a7e9d0f0bb851e38306089905d9368865bf3fd4`，tree `498d0a9235245f352bdcc738f9cfb9a364b1a9b4`，[run 36829233039](https://github.com/Naza3/Nexa/actions/runs/36829233039)，attempt1 / job `110261790429`，07:14:57 UTC启动，07:39:43 UTC终态success。全部必需步骤成功；failure-only原生诊断上传按设计skip，实际存在的三个PDB另行上传。没有创建GitHub Release、合并或部署。

### 继承环境诊断与完整回归

前置Python **38 passed，无skip**，真实Windows cmd与签名正/负用例已执行。原继承环境在系统PowerShell5.1.20348.5622显式Import Security实际失败：`System.Management.Automation.RuntimeException`，ID `FormatXmlUpdateException,Microsoft.PowerShell.Commands.ImportModuleCommand`；扩展类型`System.Security.AccessControl.ObjectSecurity`的`AuditToString`、`AccessToString`、`Sddl`成员重复。固定系统宿主、隔离子PSMODULEPATH并绝对加载系统模块后，系统文件Valid/Microsoft和未签名拒绝通过。旧失败的具体模块来源路径仍未记录，不把“PS7路径污染”升格为已确定根因。

实际环境：Windows Server2022 build10.0.20348.5622，image20260927.320.1，AMD EPYC7763 /2逻辑CPU；Rust1.98.1，CMake4.4.3，VS2022 Enterprise17.14.37710.0，MSVC19.44.35229.0，VCTools14.44.35207，SDK10.0.26100.0。原生Release/CTest、identity3项、无native管理构建、fmt、strict clippy、**234 Rust passed /0 failed /6 ignored**、五项独立旧真实GGUF及进程链路均通过。debug HTTP保持89pass/9skip、50/50/50，wrapper56.297秒、所有退出0且无强清理。上游2线程基线通过，4线程超配诊断仍failed原样保存。

### 实际交付包身份与体积

两份manifest均记录上述精确commit/tree，`project_dirty=false`、空diff hash；项目版本0.1.0、Windows x64/CPU/Release、HTTP/worker协议1、shim2、固定llama commit。模型不入包，通用context4096默认未改。

| 产物 | ZIP bytes | 解压 bytes | 内层ZIP SHA-256 |
| --- | ---: | ---: | --- |
| windows-x64-cpu.zip | 4,936,580 | 14,203,784 | `e1ee3051d06dde4a27ee722fa43911ae37657cfa55df99f7666935ede47603c1` |
| acceptance-tools.zip | 1,516,822 | 5,314,002 | `982ae4265a999cc65455f7e6590a2731a87d6d865da8cbc1e5b281dcc87e35ce` |

产品EXE分别4,456,448和6,370,816bytes，合计10,827,264bytes；产品app-local CRT共732,064bytes，UI/model为0、PDB不在包。产品manifest SHA为`ff6f560a8cbee87a022c3da0c65d1427161bf81909f9af025e46dcd942b5dce4`；工具manifest为`a9dff28d4f27f9299335401031b5b4ca94deb71f8cc7fd9b5c96a1811fed9359`。验收器EXE2,531,328bytes、SHA`5e3639a42eab3893f9ad430bfeb9b8050185158df48a37adb84b6a22cfe8a169`，与报告verifier一致。

下载后独立核对ZIP外部hash、全部manifest文件大小/hash与完整清单、SUMS映射（产品196项，工具195项）。Linux直接调用打包器最后的SUMS文本重排会遇到Windows/Linux Path排序大小写规则差异；本轮以规范化记录映射逐项复核，全部字节相同，未修改原包或重写校验文件。

产品含195项payload和186项许可原文索引；工具194项payload和190项许可原文索引。没有模型、用户data/token、测试脚本或PDB混入；已检查已知私人路径/凭据标记，不宣称这是通用未知秘密检测。项目自有许可仍仅限已授权内部开发分发边界。

### PE闭包、CRT来源与签名

产品需要`vcruntime140.dll`、`msvcp140.dll`、`vcruntime140_1.dll`；工具独立需要`vcruntime140.dll`。均来自所选VS实例`VC/Redist/MSVC/14.44.35112/x64/Microsoft.VC143.CRT/`，文件/产品版14.44.35211.0；真实系统PowerShell5.1记录`signature_status=Valid`，签名者Microsoft Windows Software Compatibility Publisher / Microsoft Corporation。没有从System32复制运行库。

| DLL | bytes | SHA-256 |
| --- | ---: | --- |
| vcruntime140.dll | 124,544 | `d5e4d9a3e835fa679450145d6a7d94e36573a509317111904d9b3712c30d9066` |
| msvcp140.dll | 557,728 | `0f885b509a685d2bbfa652fed26b5fb31d88fbdab0a978c641d1c7b8aa460aa9` |
| vcruntime140_1.dll | 49,792 | `1f2d41c4aa5db0bc33ebf7b66d72943a817d7ce6cbe880502a9403823633093f` |

下载后用独立GNU objdump重读两包共7份PE，每份AMD64、实际import集合与manifest完全相同，delay import目录均为0；app-local闭包分别完整，无工具替产品补DLL。签名Valid来自Windows实际验证记录并以对应文件hash绑定；Linux复核没有冒称独立执行Windows信任链验证。

### 真正解压Release验收

`package-acceptance.json`明确short_package_checks_passed=true，product匹配上述manifest hash，verifier匹配工具EXE hash。**16项包检查pass /0fail /2范围skipped**，嵌套HTTP **89pass /0fail /9skipped**；断流requested/attempted/passed=**50/50/50**。真实CLI初始化/重复init不轮换、启动、在线导入、列表、固定CPU/context2048/threads2/batch128/gpu0加载、worker session、卸载、生成/SSE/usage/FIFO/取消/stop均通过。

路径布尔观测确认产品EXE、源模型、自有空CWD、自有data/导入模型目录四类全部包含非ASCII和空格。源模型未修改/未hardlink，产品进程清理环境且不借工具目录DLL。服务实际退出、实例标记消失/锁释放、自有临时data清理和运行后产品完整性复验均pass。报告实际OS为RtlGetVersion build20348/product_type3/windows-server，machine-role声明ci；PATH工具不可见、开发工具/VC预装/离线字段仍unknown，A20明确unverified。

### Artifact与证据归档

| GitHub artifact | ID | 外层ZIP bytes | GitHub/下载复算SHA-256 |
| --- | --- | ---: | --- |
| 产品 | 11147880158 | 4,684,001 | `d0a20f483d785e97c4940e35134b9c78e37d3f0a74695c1048c93f674edcea39` |
| 独立工具 | 11147455846 | 1,264,785 | `71c7492f5850a32808d336806a7e45c5ce74c328a80bb33a3a04d4363a1235ea` |
| 符号 | 11147326502 | 2,907,640 | `57f31f5404c01232339002e94f611f6244225584610b82ad6d664c3ca16e7b9e` |
| 脱敏证据 | 11147735361 | 49,965 | `83b4d80fd79b062ecfefd5578df4d847f4a770e9f098bb1936a67af37b600974` |

PDB为ai_runtime3,321,856、ai_runtime_worker5,738,496、nexa_acceptance2,560,000bytes，独立保存。29份stage报告逐hash核对，真实Release验收报告SHA`be1eca1564deeb33e202e66172676a8eb9c88b61a2bb8f8d378a5d0792e22a1c`；build-result SHA`83cb216502fe24fd6f8ce49f40d8397c59811bc884f5967e9b968e8863b87216`。归档前缀`artifacts/verification/t05-windows-36829233039*`，其中`-delivery-inputs.json`列内层原包/hash/manifest/report位置，`-independent-audit.json`保存Linux逐文件/SUMS映射与独立PE import复核。

07:45 UTC，主代理独立复核后把两包文件与指南/来源组合为共395项文件的原生附件交付用户，组合ZIP SHA`d4e626d17fb4432c96c62e40dcc45add4f066a22fa13804fcf9b42e77649ae6b`；该交付记录来自主代理，不是额外一次Windows运行。交付时T05状态转为**待验证**；随后用户Windows10短验结果见下节，独立无开发工具/离线证据仍待补；Windows11、完整A01–A26、长期内存/目标性能和T06均未提前完成。

## 用户 Windows 10 / i5-8400 本地短验通过

2026-10-01，用户返回`验收报告.json`，主代理完整读取458行并核对身份；本节只记录脱敏汇总，用户原报告、完整路径与聊天不纳入Git。用户明确CPU为Intel i5-8400，报告实际观测Windows10 workstation build19044、6逻辑CPU。本次RAM未报告，不能将目标16GB写成已实测；用户明确安装过大部分开发工具、测试时有网络，VC运行库预装状态仍未知。

报告产品commit为`6a7e9d0f0bb851e38306089905d9368865bf3fd4`，manifest SHA-256为`ff6f560a8cbee87a022c3da0c65d1427161bf81909f9af025e46dcd942b5dce4`；验收器SHA-256为`5e3639a42eab3893f9ad430bfeb9b8050185158df48a37adb84b6a22cfe8a169`，均与上述交付包一致。源模型SHA-256与本记录固定Qwen3-0.6B Q8_0相同。

结果为`short_package_checks_passed=true`：**16项包检查pass /0fail /2范围skipped**，HTTP **44pass /0fail /9skipped**，断流requested/attempted/passed=**5/5/5**；退出回收、自有临时data清理和运行后产品未修改检查均pass。用户本轮执行5次，不能写成CI的50次；未记录的命令行、独立shell退出码或耗时不补造。

路径实际观测：产品EXE与源模型均含非ASCII，但**无空格**；自有CWD及data/导入模型目录均含非ASCII与空格。CI的四类中文/空格覆盖与本次目标机覆盖分别记录，不能概括为目标机四类路径均含空格。

这补齐了同一交付包在用户Win10 / i5-8400上的真实短程运行证据；A20所需无开发工具独立机器和实际离线运行仍未验证。用户随后批准延后相关验证，当前T05按缩定范围**已完成**，T06**进行中**；Windows11、长期内存/性能及完整A01–A26不由本报告推定通过。

### 用户批准延后项与阶段收口

2026-10-01 07:52–07:53 UTC，用户明确目前没有相应机器，将无开发工具、离线运行和长期稳定性作为后期验证，并要求继续开发。当前T05收口范围为已核验的Release便携包、依赖/许可/字节完整性、真实CI及用户Win10短验；这些均有对应证据，故按调整后的范围记为已完成，T06开始。A20与长期稳定性保持未验证，VC预装/RAM保持未知，不补写或修改原始报告的结果。上述各轮状态只描述当时情况，当前状态以本节及PROJECT_STATE为准。
