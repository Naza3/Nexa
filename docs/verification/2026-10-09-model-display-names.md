# 模型发现名称验证（2026-10-09）

任务W05-MODEL-NAME-1，状态：本地主机及Windows交叉联合验证通过。范围是本机/LAN `/v1/models`的兼容新增`display_name`；协议及客户端限制见[说明](../model-display-names.md)。基于2a56b396工作树已有错误处理修复继续，未提交、未推送，不把旧CI或旧二进制结果赋给本次代码。

## 已执行的针对性检查

- `cargo test --locked -p runtime-api --test management_contract v1_model_names -- --nocapture`：退出0，新增1项通过。
- `cargo test --locked --offline -p runtime-api --test management_contract --test secure_transport_contract`：退出0，管理11项、传输24项通过/1项既有忽略。
- `node /workspace/shared/nexa-tooling/provenance/model-display-name/verify-pi-consumer.cjs`：退出0。通过GitHub读取官方PI Desktop v0.17.0源码，使用现有TypeScript编译器提取原AST并实际运行解析函数、设置过滤回调与名称JSX、聊天别名函数；未改写所测函数算法。覆盖中文/空格/符号、不同ID重名、缺失及空名称回退、重复ID去重、按名称/ID搜索、React文本转义与别名保留ID。该隔离源码测试不是完整IPC或已安装Electron GUI验收。
- `cargo test --locked --offline -p runtime-api`：退出0，109通过/1项既有忽略；72+2+11+24为各测试程序结果，包含上述定向子集，不再叠加。
- `cargo fmt --all`：退出0，随后联合门槛执行`--check`。

HTTP契约测试实际走router/鉴权/actor/store：旧ID加载成功，同名模型分别保留，本机分页ID不变，LAN只列驻留项，卸载后为空；专用无注册表的合成传输fixture验证名称回退。合成executor不构成真实模型推理质量证据。

原始证据目录：`/workspace/shared/nexa-tooling/provenance/model-display-name/`；`focused-api.log`、`pi-consumer-result.json`及官方源码快照/验证脚本保存在该处，不把第三方源码副本加入产品。

## 最终源码联合门槛（全部通过）

本轮将前轮[完整错误处理](2026-10-09-error-handling.md)与模型显示名变更一起重新验证，使用独立`nexa-tooling/provenance/model-name-combined-final/`证据目录，旧错误处理收据保持不变。

| 实际门槛 | 当前结果 |
| --- | --- |
| 根workspace及桌面manifest `cargo fmt -- --check` | 退出0 |
| `cargo test --locked --offline --workspace` | 退出0，653通过/10既有忽略 |
| 根workspace `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` | 退出0 |
| 桌面manifest host tests与all-targets strict Clippy | 均退出0，33项通过 |
| 前端 `npm run typecheck`、`npm run lint`、`npm test`、`npm run build` | 全部退出0，49文件1006项通过 |
| 严格Python全量（EncodingWarning视为错误） | 退出0，366项：361通过/5平台skip |
| native_identity直接Rust测试 | 退出0，5项通过 |
| Windows固定native缓存与两workspace Release strict Clippy | 均退出0 |
| Runtime/worker/acceptance/desktop四EXE Windows实际链接及PE/import复核 | 退出0，四EXE均AMD64 PE32+，10份native库身份通过 |
| 前轮安装helper身份 | 源码SHA与已验产物不变，复用前轮三helper交叉结果；未重新打安装包 |

源码冻结后捕获本轮source-receipt和非Markdown代码hash：基线commit为`2a56b39602806fbaf3c74841a5a8b0d1534bcf77`，tree为`3a3db727e5b78b1bf353319f06c72e3dd6d9e2a7`，dirty=true，物化588文件。构建后442个非Markdown文件hash完全一致，结果文档追加不冒充新提交。旧commit/旧CI不能替代本轮dirty源码验证。

## 未验证边界

Windows原生CI、用户PI Desktop实际版本/GUI与新包运行均未由本记录证明。没有修改模型ID或为第三方客户端自动设置别名。PI证据是官方函数/JSX隔离执行，Nexa证据是实际HTTP router+鉴权+actor/store及合成executor；两者不等于完整IPC、真实模型、Electron GUI或目标Win10验收。

未提交、未推送、未创建tag/Release；下一步由用户决定交付，不自动沿用上一轮发布授权。


## 最终证据与已知warning

本轮无新增测试/编译失败。切片109项、定向35项不叠加到最终根653项。原始日志、`source-receipt.json`、`source-code-before.json`、`cross-result.json`及最终完整patch均保存在`nexa-tooling/provenance/model-name-combined-final/`；本机路径不作为用户下载链接。根与桌面host fmt/Clippy、两Windows strict Clippy、构建命令最终退出码均0，空白及文档链接复核通过。

前端保留大chunk提示；桌面Windows保留clang-cl compiler-family探测与SDK CRT缺PDB的LNK4099调试信息warning。未关闭lint、未改工具链来源或绕过安全检查；warning不冒充Windows运行通过。

### 四EXE SHA256

- `runtime`：`7c77b26ebafc97653fa2b6524af043b90d9098d073c0f598f71e765b7180be3c`（6227456字节）
- `worker`：`c422c586f70b5069053ab0cb8dc82c46d1ab032d0289cd774d7a678d0a3fd19f`（8711680字节）
- `acceptance`：`5457b0016f3baa28b5ea783af8c9390f55c254a3b5b0b3b0c9355c4b46c1fdfd`（2654208字节）
- `desktop`：`ac59ace5de4156d9b4d498eea523f175c80a6e28d9328771a73f73ced9eda83e`（15616512字节）
