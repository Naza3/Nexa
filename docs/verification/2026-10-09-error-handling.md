# 2026-10-09 错误处理联合验证

任务W05-ERROR-1 / W05-LAN-RECOVERY-1。工作树在`codex/dev`基线`2a56b39602806fbaf3c74841a5a8b0d1534bcf77`上修改，未提交/推送。原上一轮三份未提交证据文档保留；本轮新行为见[错误恢复契约](../error-handling-and-recovery.md)。

## 范围与验证计划

包含LAN降级/恢复、启动私有stdout固定码、配置有界原因、统一前端脱敏与显式恢复、下载清理和发布结果待确认、安装helper固定诊断。独立审查未发现阻断；审查不代替运行验证。

源码冻结后依次验证根workspace/桌面壳fmt、全量Rust测试与strict Clippy；前端typecheck/lint/全部测试/build；严格Python全部测试；锁定Windows native静态库、两个workspace Windows strict Clippy及Runtime/worker/acceptance/desktop四EXE实际链接、AMD64 PE与原生身份复核。验证期间只允许针对失败的最小修复，再复跑适用门槛。

## 执行记录（最终全部通过）

| 门槛与实际命令 | 当前结果 |
| --- | --- |
| `cargo fmt --all -- --check`、桌面manifest同命令 | 退出0 |
| `cargo test --locked --offline --workspace` | 退出0，652通过/10既有忽略 |
| `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` | 退出0 |
| `cargo test --locked --offline --manifest-path apps/desktop/src-tauri/Cargo.toml` | 退出0，33项通过 |
| 桌面manifest `cargo clippy --locked --offline --all-targets -- -D warnings` | 退出0 |
| `npm run typecheck`、`npm run lint`、`npm test`、`npm run build` | 全部退出0；49文件1006项通过 |
| `PYTHONWARNDEFAULTENCODING=1 python3 -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'` | 退出0，366项：361通过/5平台skip |
| `rustc --edition=2024 --test crates/llama-adapter/native_identity.rs`及生成测试程序 | 退出0，5项通过 |
| 固定CMake Windows原生静态库配置/构建 | 退出0，复用有效native缓存；10份静态库身份复核通过 |
| 根workspace Windows Release strict Clippy、Runtime/worker/acceptance实际链接 | 均退出0 |
| 桌面workspace Windows Release strict Clippy | 退出0 |
| 桌面Windows Release EXE实际链接及四EXE PE/原生身份复核 | 退出0，四EXE均AMD64 PE32+，源码及native身份通过 |
| 三个安装helper `/W4 /WX`实际交叉链接 | 原作者退出0；联合复核源码hash、新check EXE hash及三个AMD64 PE/import通过 |
| `git diff --check`及新增文档链接检查 | 退出0（最终复核） |

计数不叠加切片或定向子集。Windows helper与四个产品EXE是独立构建产物，不把静态helper测试算为安装器原生运行。前端build保留既有大chunk提示；桌面Windows构建保留clang-cl compiler-family探测warning，并出现SDK静态CRT缺少PDB的LNK4099调试信息warning；实际链接退出0，未关闭Clippy门槛。

### 联合门槛发现并修复的问题

1. 首次严格Python运行366项，新增安装诊断夹具两个subprocess文本调用未指定encoding，触发EncodingWarning错误。仅补UTF-8参数后完整366项重跑通过。
2. 首次根workspace编译发现xtask的错误诊断测试构造`ClientError::Api`漏新增`reason`字段；只补`reason: None`，再跑根全量652通过/10忽略。切片测试不能覆盖这处，因此保留首次失败日志。

### 源码与产物证据

在所有代码修复后捕获`source-receipt.json`及`source-code-before.json`：基线commit为`2a56b39602806fbaf3c74841a5a8b0d1534bcf77`，tree为`3a3db727e5b78b1bf353319f06c72e3dd6d9e2a7`，工作树dirty=true，物化585文件。不能把基线commit或旧CI宣称为本次修改的干净源码身份。构建后复核442个非Markdown文件hash完全一致；文档结果追加不冒充已提交源码。

完整日志与源码/产物证据位于本次执行环境`nexa-tooling/provenance/error-handling-final/`；源码hash、PE结果和二进制SHA写入该目录的JSON证据。安装器原始日志在`installer-diagnostics-20261009/cross-helpers.log`，本轮复核记录为`installer-helper-verified.json`。这些本机路径不是用户可下载链接。

## 边界

主机测试不等于Windows运行；交叉链接不等于原生CI或真实推理。真实模型依赖测试按原条件忽略，须单独记录。新原生Windows安装、GUI、保留服务后关闭桌面/外部客户端调用、目标Win10/i5与真实LAN可达性未执行。未创建新安装包/正式Release、未推送或修改tag，不把旧包授予新行为。

## 四EXE交叉产物SHA256

- `runtime`：`6c83b6ccd7148001d81878a9317b88aff85c31c426d7832e267581eaa597eefc`（6240256字节）
- `worker`：`b8d175797dff253a7a727eb1e7adce9b37c5df59c79acd1b7eadc7a1c6ef4505`（8711680字节）
- `acceptance`：`afcaca357e84a274a9481323b0d2691e9c12032e59dda0f4ff2046976a75f53d`（2654208字节）
- `desktop`：`7d332003546ee6c0fc92309a0acb70d113a89e2730eb640104296294f9685064`（15616512字节）

整体本地验证状态：已完成。下一步为父任务决定后续开发/交付；不自动提交或推送。后续代码修改须重跑适用联合门槛，不能沿用本轮代码hash证明。

## 后续组合源码验证

随后加入最小模型发现`display_name`增量，完整门槛再次通过；最新653项根测试及四EXE身份见[模型名称联合记录](2026-10-09-model-display-names.md)。本页652项与旧patch继续表示名称增量前的已验切片，不将其复用为最新组合源码收据。
