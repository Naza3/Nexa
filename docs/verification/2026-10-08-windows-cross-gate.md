# Windows Clippy 修复与推送前交叉构建

任务 W05-CI-FIX-3 / W05-CROSS-2。用户在构建失败后要求本地编译Windows版本，并明确更正顺序为：先交叉编译和检查通过，再推送GitHub，最后原生Windows构建与运行测试。不是推送后并行补验。产品版本保持0.2.3，分支沿用codex/dev，已包含当时最新main。

## 原失败与修复

[Actions37716280319](https://github.com/Naza3/Nexa/actions/runs/37716280319)的桌面job在`windows.rs:1009`报`clippy::needless_borrows_for_generic_args`。锁定Tauri 2.12.1的`eval(&self, js: impl Into<String>)`接受String；将`eval(&format!(...))`改为`eval(format!(...))`，脚本及关闭保存行为保持。既有`-D warnings`继续生效。

该源码的前端45文件960项与Windows壳37项通过，Runtime、下载组件和版本job成功；桌面失败导致最终native与release job跳过，不将部分成功写成整轮通过。Windows专属模块不参与Linux主机Clippy，这是旧检查没有发现问题的原因。独立只读审查核对官方Tauri crate字节与Cargo.lock SHA、关闭nonce/ACL/注册流程，无新增阻断。

修复后本地桌面主机fmt、Clippy及diff检查退出0（日志`/tmp/nexa-windows-eval-clippy.log`），仅作为主机检查。Windows目标验证另行记录。

## 本机工具准备

- Rust/Cargo 1.98.1与`x86_64-pc-windows-msvc`标准库；CMake4.4.4、Ninja1.13.2、Node24.19.0/npm11.9.0沿用已有开发环境。
- Clang/LLVM/LLD19.1.7（Debian13官方包）、cargo-xwin0.23.1（项目PyPI wheel）；工具独立放在`/workspace/tooling/windows`。Debian InRelease签名、索引/包SHA256与PyPI wheel SHA256逐层核验；没有全局特权安装或关闭TLS。
- Microsoft SDK请求版本10.0.26100、CRT清单键14.44.17.14，通过cargo-xwin从官方目录准备。最初将CI的Redist版本14.44.35112当作清单键而失败；查实际目录后改用对应工具集键，保留首次错误日志，没有修改校验值。
- 工具刷新脚本`/workspace/tooling/windows/install-tools.py`重跑退出0，来源与无SDK Windows COFF/PE/资源编译烟测见`/workspace/tooling/windows/provenance/validation.txt`。该烟测不是Nexa编译或Windows执行证明。

SDK来源独立复核：40个下载payload的SHA256均匹配经官方HTTPS目录取得的记录；目录大小字段仅37/40一致，3个CRT VSIX不符。channel声明vsman为`6e470016e4324c84c255ffd0beb3767d17ec89cc8561e9409ee3e1f6d29400f5`/30,443,537字节，实际为`f0a50ea157222c29abd5ea6ff01bfc3c33b04e011c5e45ee2ca38ef0778e5643`/17,954,732字节。独立审查核对cargo-xwin所用xwin0.10.0官方源码：`manifest.rs`明确目录摘要长期不正确，默认不校验该级摘要；payload下载仍严格校验期望SHA。未修改工具、期望摘要或关闭现有校验。来源信任限于官方URL/TLS与随后40个payload SHA，不宣称完整channel摘要链、全部大小或数字签名通过。结果保存在`/workspace/onboarding/windows-cross/sdk-payload-verification.json`，不影响原生CI的独立工具链验证。

## 推送门槛与验证边界

规则写入AGENTS、构建锁与[交叉构建说明](../windows-cross-test-build.md)。本机复用入口为`bash /workspace/onboarding/windows-cross/build.sh`：捕获源码收据，构建前端与Clang MSVC ABI `/MD`原生库，两个workspace执行Windows Release strict Clippy并实际链接四个EXE，最后复核同一源码、所有阶段退出码、原生库身份与AMD64 PE32+导入目录、大小/SHA256。独立脚本控制流审查注入各阶段失败，确认不会误报成功。

构建日志和结果写入`/workspace/onboarding/windows-cross/runs/`，不提交SDK、生成文件、二进制或原始日志。只有本次完整脚本退出0才允许推送。此步骤不运行Windows程序，也不产生完整安装包；原生CI、安装器、WebView2及用户Windows10/i5目标机结果仍须分开记录。

## 本次实际结果

SDK首次准备成功，17分49秒，慢在实际CAB解压；完成后缓存保留。SDK尚未就绪时尝试提前配置原生库，因kernel32等系统库缺失退出1，未继续编译或放行；完成SDK后正式配置成功。`bash /workspace/onboarding/windows-cross/install.sh`重复执行退出0、约0.5秒，确认已完成缓存可复用。

正式门槛命令`bash /workspace/onboarding/windows-cross/build.sh`退出0；证据目录`/workspace/onboarding/windows-cross/runs/20261008T024028Z`。源码收据以`f9bf46d21b517fd64c47a5bce7f614a747aee72a`加已记录工作树改动捕获，构建前后完整相等，未伪称干净提交。唯一产品代码改动是`windows.rs`一处引用移除，其SHA256为`b128e4f729b661546df8043aca4e62b708ebf98661481f7a80a39dfcd00ff86b`；完成后仅补本轮验证文档，不改变已编译代码。

| 阶段 | 实际结果 |
| --- | --- |
| 前端生产构建 | 退出0；既有大于500kB chunk提醒保留 |
| Windows原生库配置/编译 | 退出0；189个Ninja任务，10项静态库闭包，MSVC ABI、Release `/MD`及AVX2基线通过 |
| 桌面Windows Release all-targets strict Clippy | 退出0，2分03秒 |
| 根workspace Windows Release all-targets strict Clippy | 退出0，36.21秒 |
| 桌面Release实际链接 | 退出0，5分24秒 |
| Runtime/worker/验收器Release实际链接 | 退出0，2分04秒 |
| 收据、全部阶段退出码、原生库身份、四EXE AMD64 PE32+及导入目录 | 退出0 |

桌面交叉构建仍打印既有clang-cl编译器家族探测warning，实际资源与EXE链接成功；没有屏蔽Rust lint或放宽Clippy。阶段并行，表中耗时不能相加成总耗时。普通主机/原失败run测试计数不重复计入此次交叉编译，没有在Linux运行Windows测试。

| EXE | 字节数 | SHA256 |
| --- | ---: | --- |
| ai-runtime.exe | 6257664 | `9120650891f5553f00bb186c425f5bf22075bc39fd39dbe9f80557f7df0e7161` |
| ai-runtime-worker.exe | 8707072 | `164acd6f0a74653fbc3b2cd4a7238313958e097666f9a6bcbb08e4f4ba96f62a` |
| nexa-acceptance.exe | 2651648 | `72486d6bbedfe443d296ef1d3e6116c37c4d8f780f4507d5147ccd138b49acd7` |
| nexa-desktop.exe | 15249408 | `ced926f0bf04a19b6d26ff19a0563c50a44b224429c9fc01d576e0757c1111a8` |

源码、主机检查与本地Windows交叉门槛已完成；推送后的原生CI结果见下方独立记录。

## 用户下载的交叉便携测试包

用户随后要求下载本机构建结果。为绑定可审查的干净源码，在已推送的`0ecddad0b837a9ea5571b01bc28eea73461357d1`上重新捕获收据并执行完整门槛，退出0；证据`/workspace/onboarding/windows-cross/runs/20261008T025622Z`。此轮在首次推送前检查已通过之后，属于新增下载任务的干净提交打包验证，不替代先验后推送顺序。

使用生产`package_windows_cross_test.py`生成完整便携测试ZIP，退出0。Runtime/worker/桌面/验收器为本地交叉编译；aria2取自[本次同提交CI](https://github.com/Naza3/Nexa/actions/runs/37719780731)的`nexa-download-source-build-0ecddad0b837a9ea5571b01bc28eea73461357d1` artifact，重新核对完整原始清单并按生产准备入口生成组件。没有改旧manifest冒充同源，也没有加入模型、测试EXE或原始构建日志。

官方CRT原VSIX及原许可DOCX保留来源、SHA和实际字节。Debian签名索引校验的osslsigncode2.9原在线CRL请求因微软Content-Type与OpenSSL不兼容失败；随后经官方HTTPS取得原始CRL，使用工具支持的`-CRLfile`及`-TSA-CRLfile`参数验证。wrapper仅追加这两个参数、不过滤输出、不禁用签名/时间戳/吊销检查。生产验证函数对11个Release DLL各两份签名及其时间戳/CRL通过；包中按实际导入闭包仅复制3种DLL、4个文件。未宣称Windows `Get-AuthenticodeSignature`通过。工具、证书、CRL及失败/成功日志见`/workspace/tooling/windows/packaging/`。

最终ZIP39文件，App含runtime/download在内的许可文件恰为10份；文件清单、大小/SHA、普通与delay PE导入、ZIP逐字节、源码收据复核通过，真实Rust layout和download identity消费者在Linux上读取最终包通过。独立验收器只随包提供，没有执行Windows程序。

- 路径：`dist/Nexa-0.2.3-windows-x64-cross-0ecddad.zip`
- 大小：18,721,201字节
- SHA256：`b62590ffb0c33887619d08cdaecadce6d0664c44c6d9ad65fb518dd6918f31aa`
- 对应`.zip.sha256`一同生成；完整记录在`dist/Nexa-0.2.3-cross-0ecddad/build-result.json`与`/workspace/onboarding/windows-cross/package.log`
- 启动：退出旧Nexa，解压后运行`Nexa-Windows-cross-test/desktop-windows/nexa-desktop.exe`；需已有WebView2，不包含模型权重

此ZIP明确为Linux交叉编译的便携测试包，不是GitHub原生构建包，也不是正式Release。独立只读交付复核确认ZIP与sidecar SHA相等、启动EXE存在、AVX2基线适配i5-8400；用户Windows10/i5目标机实测仍独立待验。

## 同源码原生Windows CI

本地完整门槛通过后，提交并推送`0ecddad0b837a9ea5571b01bc28eea73461357d1`，tree为`4daa3be028675c2b2dd38f38622d8b3de08b2ce4`。[Actions37719780731](https://github.com/Naza3/Nexa/actions/runs/37719780731)于2026-10-08 03:15:07 UTC完成，整轮`success`；`gh run watch --exit-status`退出0。release-identity、desktop-build、download-component、runtime-build与native五项成功，非tag分支的release按条件跳过。

- 前端45文件960项与独立Windows桌面壳37项通过，原失败的Windows strict Clippy通过，桌面Release实际链接成功。
- Windows根workspace Rust55组625通过、0失败、10既有忽略；随后显式运行真实模型/Runtime/worker信用测试，另记其独立验证，不混入普通用例总数。
- Python319项（317通过、2平台skip），原生CTest5/5；真实模型、HTTP/CLI生命周期、下载组件、独立解压桌面包与安装器生命周期步骤均成功。
- 原生MSI构建报告确认payload与MSI表校验、Setup内嵌精确MSI通过；产物保持未签名。[三格式CI产物](https://github.com/Naza3/Nexa/actions/runs/37719780731/artifacts/11526615269)已由工作流保存，未发布为公开Release。

原始job日志保留在`/tmp/nexa-37719780731-desktop.log`、`/tmp/nexa-37719780731-runtime.log`、`/tmp/nexa-37719780731-native.log`；安装诊断在`/tmp/nexa-37719780731-installer/`。这些原生运行结果只对应CI生成的二进制；本地交叉ZIP只完成交叉构建与打包/静态校验，不能声称它在Windows运行通过。Windows10/i5实际GUI、真实长图质量和长期使用仍待用户设备验证。

交付前再次fetch main并确认已包含，纯文档记录不改变产物源身份、不重复全量构建。云环境install_script/start_skill已保存为配置草稿，内容指向实际测试过的工具准备与门槛脚本；尚未发布环境快照，不宣称后续新环境恢复已验证。用户可在环境设置中审阅、保存并发布以复用。
