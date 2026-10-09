# 启动声明文件校验调整

任务 W05-LAYOUT-1；2026-10-09。状态：源码、本机测试与完整Windows交叉门槛通过，原生运行待验证。下方保留先前工具缺失阶段与后续验证结果。

## 基线与范围

真实仓库 `Naza3/Nexa`，长期 `codex/dev`，基线 `68fcc0d194fdd9d943f95ed89600dec59317a934`；其祖先含当前main `1c3650c352e56472f6ec7a5880b519f905ea6f8a` 和v0.3.0源码 `a7db515f649f8ba05f3837c050b6ed56b920968c`。仅本地工作树修改，版本0.3.0不变，无提交/推送/tag/Release。

按用户“不需要未声明文件的检验”，删除桌面启动layout的递归库存与未知内容拒绝，保留声明文件大小/hash、清单/SHA256SUMS、来源匹配及普通文件/祖先间接路径防护。显式要求固定可执行文件列入其产品清单，避免删除精确库存后产生“文件存在但未hash”的缺口。模型准入、ACL、token、数据配置与独立下载组件执行校验不变。决策见[ADR0041](../decisions/0041-declared-payload-startup-validation.md)。

## 第一阶段回归与证据

- Rust新增/更新：uninstall.exe及其他额外文件、任意嵌套目录允许且不修改；声明EXE同大小篡改与缺失拒绝；自洽manifest/SHA256SUMS漏掉必需EXE拒绝；无关dangling/cycle链接不遍历而声明文件或祖先链接拒绝；Windows junction对应正负例；包内pick/apply/rescan允许额外内容但不允许声明payload篡改
- Python smoke将未知DLL从负例改为正例，仍检查身份不变与manifest篡改拒绝，保留独占创建/清理及payload恢复
- 原生MSI/NSIS生命周期在原版安装后、标记安装门槛成功前运行已安装EXE的 `--diagnose`。失败仍保存闭合诊断；身份须与原版manifest一致。报告写入既有 `artifacts/verification/windows-desktop/diagnostics-installed-{msi,nsis}.json`，纳入现有证据归档；未修改17项生命周期schema
- 独立审查指出并已修复：必须显式列入清单的EXE检查、安装诊断写入目录与stager一致。旧“四字节头/未知内容拒绝”契约与注释已同步

实际命令及结果：

| 命令 | 结果 |
| --- | --- |
| `git submodule update --init --depth 1 vendor/llama.cpp` | 退出0；固定2149c00f4442dc59302e134a02e4c99d5f7ed9fc |
| `python3 -m unittest discover -s scripts -p 'test_desktop_smoke.py' -v` | 退出0，30项 |
| `python3 -m unittest discover -s scripts -p 'test_tauri_lifecycle_contract.py' -v` | 退出0，14项 |
| `python3 -m unittest discover -s scripts -p 'test_stage_ci_evidence.py' -v` | 退出0，23项 |
| `PYTHONWARNDEFAULTENCODING=1 python3 -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'` | 退出0，364项：359通过/5平台跳过；上述定向项是子集，不重复计数 |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml` | 退出127：cargo未安装，测试未执行 |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --check` | 退出127：cargo未安装，格式门槛未执行 |
| `git diff --check` | 退出0 |

全量Python首次运行因尚未初始化llama.cpp子模块，三个许可测试报原件缺失；补齐锁定子模块后重跑通过，没有削弱测试。原始本地证据保存在忽略目录 `artifacts/verification/declared-payload/`，不将工具、原始日志或二进制加入源码。

## 第一阶段待验与下一步（已由下方追加结果更新）

本环境没有Rust、Windows交叉工具链和此前缓存。Rust回归、rustfmt/Clippy、原生推理库与四EXE链接尚未执行；新增Windows junction和已安装EXE诊断也未运行。必须按[交叉门槛](../windows-cross-test-build.md)准备官方工具和SDK，在相同代码上通过本地完整Windows门槛后才可推送，再做原生CI与用户Windows10验收。

`--diagnose`只检查包与WebView2可用性，不创建WebView、不初始化配置或启动runtime。本修改不证明MSI的独立 `configuration_unavailable` 问题已修复，不以Python或源码审查替代Windows运行验证。

## 追加：隔离工具准备与完整Windows交叉门槛通过

用户确认安装官方工具并接受[微软相关条款](https://go.microsoft.com/fwlink/?LinkId=2086102)后，全部工具仅写入云端工作目录，无系统级/管理员安装，无用户电脑变更。

工具来源与边界：

- Rust/Cargo1.98.1、rustfmt、Clippy、Linux host与x86_64-pc-windows-msvc标准库，下载自官方static.rust-lang.org；六个组件逐项匹配官方发行清单SHA256
- LLVM/Clang/LLD19.1.7来自Debian13官方包；系统自带Debian keyring验证InRelease签名，再核验Packages索引及各下载包SHA256。仅解包到隔离目录，未运行系统包安装
- CMake4.4.4、Ninja1.13.2、cargo-xwin0.23.1使用对应官方项目的PyPI wheel；保存来源URL、版本和摘要。Node24.19.0/npm11.9.0沿用环境
- Microsoft SDK10.0.26100与CRT工具集14.44.17.14由未修改的cargo-xwin准备，成功耗时14分44秒。首次客户端连接失败；curl同一官方地址可达，明确让ureq使用环境已配置的同一HTTP代理后恢复，未绕过网络限制或关闭TLS
- 40个SDK/CRT下载payload独立核验SHA256全部匹配，37/40声明大小一致。三个CRT VSIX大小元数据差异及channel→vsman摘要缺陷与前次记录一致；xwin0.10.0上游本来不核验该级vsman摘要，未修改工具或预期摘要。实际vsman SHA256 `f0a50ea157222c29abd5ea6ff01bfc3c33b04e011c5e45ee2ca38ef0778e5643`，channel声明 `6e470016e4324c84c255ffd0beb3767d17ec89cc8561e9409ee3e1f6d29400f5`。信任限于官方HTTPS与payload摘要，不宣称完整目录摘要链或Windows签名验证。官方工具完成后自动清理下载缓存；独立核验记录在清理前已保存

实际新增命令与结果（全部退出0）：

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml`，随后相同命令加 `--check`；根 `cargo fmt --all --check` | 格式化并复验通过 |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml` | Linux壳33项全部通过，含本轮新增回归 |
| `cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings` | Linux壳严格Clippy通过 |
| `npm ci`、`npm test -- --run`、`npm run typecheck`、`npm run lint`、`npm run build` | 46文件970测试通过，静态检查与前端构建通过；首次npm默认缓存不可写，改为隔离缓存后成功 |
| 严格完整Python重跑 | 364项：359通过/5平台跳过 |
| `rustc --edition=2024 --test crates/llama-adapter/native_identity.rs` 后运行生成测试程序 | 5项通过 |
| `bash /workspace/shared/nexa-tooling/build-cross.sh` | 完整Windows交叉门槛退出0，具体阶段如下 |
| CMake Windows x64/Clang MSVC ABI/Release `/MD`配置、固定AVX2探针及 `cmake --build ... --target air_llama --parallel 2` | ABI/基线验证通过，189/189编译任务完成，10项静态库闭包 |
| 根 `cargo xwin clippy --locked --release --target x86_64-pc-windows-msvc --workspace --all-targets -- -D warnings` | 通过，35.97秒 |
| 根 `cargo xwin build --locked --release --target x86_64-pc-windows-msvc -p runtime-cli -p runtime-worker -p xtask` | 实际链接通过，1分18秒 |
| 桌面 `cargo xwin clippy --locked --release --target x86_64-pc-windows-msvc --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings` | 通过，1分29秒 |
| 桌面 `cargo xwin build --locked --release --target x86_64-pc-windows-msvc --manifest-path apps/desktop/src-tauri/Cargo.toml` | 实际链接通过，3分40秒 |
| 生产 `native_identity` / `pe_machine` / `parse_pe_inspection` 与源码收据复验 | 10个静态库身份正确；四个必需EXE均AMD64 PE32+、导入目录解码一致、无延迟导入；冻结源码全程一致 |

保留已有第三方C++ deprecated警告、clang-cl编译器家族探测warning，以及桌面链接的缺少CRT PDB调试符号LNK4099警告；没有屏蔽Rust lint或修改链接器策略。所有要求的实际链接均退出0，PDB警告不冒充零警告。

| 必需EXE | 字节数 | SHA256 |
| --- | ---: | --- |
| ai-runtime.exe | 6263808 | `e62ce36d27ee2ad470a745aa75c27fbeae55fffde1c9fb181b37fc9f7b2bbdc7` |
| ai-runtime-worker.exe | 8711680 | `0b6855c7c7204c8738901d4b3433e794f7baa1f0712b7c9a7676f1cb774d5aaa` |
| nexa-acceptance.exe | 2655232 | `16aff984a4e5f1f0cf1950fad8d78a98aa64f5d38c9dd5cef0e20317da5bea2c` |
| nexa-desktop.exe | 15578112 | `d76f08a429c78aaf60cd8782ec5c0778460d38b77e1a770ce07ddee78ce12260` |

证据在 `/workspace/shared/nexa-tooling/provenance/`，完整构建入口与隔离工具环境分别为 `build-cross.sh`、`env.sh`。`cross-result.json`关联四EXE、原生库和构建前后相等的源码收据；构建基线为上述68fcc0d加本地修改，dirty=true，573份物化源文件。冻结工作树文件摘要为 `42e128d116e0ea0281e5fefb4672ec058c989f91730dc313b998d61b0c185b00`。本节与状态的后续纯文档补记不改变已经验证的Rust/脚本代码；不能将这些本地产物说成未经创建的新提交产物。

结论：源码、本机测试和完整Windows交叉门槛通过。仍未运行Windows程序、MSI/NSIS安装诊断、原生窗口、真实模型或用户目标机；MSI独立configuration_unavailable仍未由此证明解决。版本0.3.0保持，当前未提交/推送/tag/Release；待用户明确批准推送后运行原生CI。

## 推送授权与后续门槛

用户随后明确要求“推送到GitHub 然后构建”。重新fetch确认远端codex/dev仍为68fcc0d、main仍为1c3650c，工作树17文件与交接摘要一致，无他人改动。按既定顺序：精确暂存本批文件并中文提交，在该干净提交上重新记录源码收据、复跑缓存Windows交叉门槛，成功后才正常推送codex/dev，由既有分支工作流执行Windows原生验证。未授权tag、main合并或正式Release发布；后续原生结果另据对应提交与run记录，不把本段计划写成已经推送。
