# 2026-10-10 工具调用兼容验证

任务 W04-TOOLS-1，基线为 codex/dev 的本地 Rust 错误规范提交 c8cf4f7；本轮不自动推送、合并 main 或发布安装包。公共版本仍 0.3.0。范围与决定见 [ADR0044](../decisions/0044-openai-tool-calling-compatibility.md)。

## 实现范围

OpenAI tools/tool_choice/parallel_tool_calls、assistant.tool_calls 与 tool 结果历史；流式及非流式响应；原始模型模板的严格解析；全量 prompt token 预算；工具增量与普通文本共用一次性信用、取消与终态。私有 IPC/shim 身份同步为 5，新增 v3 C ABI，旧文本/OCR入口不变。客户端负责工具授权与执行。

工具输出先完整验证再分片，未声称实时参数流或 JSON Schema 约束解码。不完整/无效/超限输出不生成成功工具终态，不进行自动重放。参数 schema 元数据保留完整有界结构，strict:true 明确拒绝。

## 环境与已经通过的检查

Linux 云端，Rust 1.98.1、CMake 4.4.4、Ninja 1.13.2；llama.cpp 固定 2149c00f4442dc59302e134a02e4c99d5f7ed9fc。不是 Windows 构建或用户设备验收。

- `cargo test --locked --workspace --all-targets`：退出 0，689 通过、11 既有 ignored；原生库已构建并实际链接。日志 `/tmp/nexa-tools-full-host-final.log`。
- `cargo fmt --all -- --check` 和 `cargo clippy --locked --workspace --all-targets -- -D warnings`：退出 0；日志 `/tmp/nexa-tools-full-clippy-final.log`。
- `python3 -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'`：退出 0，372 项中 367 通过、5 平台 skip；日志 `/tmp/nexa-tools-python.log`。
- 原生 CTest 5/5；adapter 13、EngineHost 2 单测及两个 crate strict Clippy 通过（上述 Rust 子集不重复加总）。日志位于 `/workspace/shared/nexa-rust-tooling/logs/tool-*.log`。
- 锁定 PI Desktop v0.17.0 所用 pi-ai 1.0.1 加官方 patch：5 个真实 serializer/parser 合成 HTTP 场景和 3 个成功终态执行门测试通过；在新空目录重建客户端独立复现通过。它不是 PI Electron 窗口或完整 agent 的运行结果。
- 根及独立桌面 Cargo.lock 仅给 runtime-types 补入已锁定的 serde_json 依赖，不升级第三方版本。独立桌面首次 --locked 检查揭示第二份 lock 缺此依赖，已同步后重跑，随后 `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml` 33 通过，独立桌面 fmt/strict Clippy 退出 0；日志 `/tmp/nexa-tools-desktop-{test,clippy}.log`。

## 审查发现及修复

1. IPC codec 的信用白名单起初未包含工具 delta，已修复并以实际编码/读取测试覆盖。
2. 携带工具保留预算的终态在满队列取消时可能被误当 payload 丢弃；现在仅 text/tool delta 可在取消时丢弃，终态必须交付，新增 rendezvous 满队列回归。
3. shutdown/fault 也必须把保留预算随终态交给 actor，不能在长期校验副本销毁前提前释放。
4. 不均匀参数追加的 String capacity 翻倍会超过声明预算；改为精确扩容，并检查实际 id/name/arguments capacity 总量，4 调用多段追加回归通过。
5. 非流式最坏 JSON 转义扩大：4 个合法参数各约14KiB、总量小于64KiB，最终编码超过96KiB；完整HTTP回归确认非流式明确 `response_too_large`，无choices/tool_calls/成功尾，同一数据的SSE完整重组4调用且只有一个DONE。该新增回归和strict Clippy通过。

工具账本 768KiB，长期输出副本保留 384KiB；其余信用、scratch、最终编码缓冲总和 736KiB。此处不包含整个模型 RSS、模板/PEG/JSON 解析临时工作区，不声称进程级硬内存限制。

## 真实模型分层证据

### Qwen3-0.6B-Q8_0

- 固定官方来源 Qwen revision 23749fefcc72300e3a2ad315e1317431b06b590a。
- GGUF SHA256 `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`，639446688 字节。
- 模板 SHA256 `57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361`。
- 真实 adapter 两轮内存查表→结果→文本回答通过；第二轮移除工具定义仍保留合法工具历史通过；max_tokens=1 截断零发布事件、完整工具提示词超预算负例通过。
- 完整 PI→HTTP→worker→真实模型闭环也通过：两次模型请求、一次成功终态后的内存查表；首轮 toolUse，次轮 stop 且三个固定结果锚点匹配。实际 prompt/output token 为227/22、280/19，耗时30.1秒，无重试、强制清理或系统工具执行。证据 `/workspace/shared/pi-tools-validation/real-qwen06.json`；实际握手 protocol/shim=5。

### Qwen3.5-4B-Q4_K_M

- [固定目录样例来源](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/e87f176479d0855a907a41277aca2f8ee7a09523/Qwen3.5-4B-Q4_K_M.gguf)。用户只口述 Qwen3.5-4B-Q4，尚未确认其文件与此样例相同。
- GGUF SHA256 `00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4`，2740937888 字节。
- 内嵌模板 SHA256 `7f0e529032c25183bcd66c7f238da2d377f43be754a94e2725a58c4e16d2ed67`，7816 字节。
- 上游源码模板及该 GGUF 实际内嵌模板的严格 PEG/typed XML 参数正反例通过，覆盖中文、换行、嵌套 JSON、数字和缺失闭合。此探针与下项完整模型推理分别记录。
- 完整 PI 客户端→HTTP→worker→真实4B两轮闭环通过：首轮 toolUse，成功终态后只执行一次固定内存查表，按 call_id 回传后次轮 stop 且三个结果锚点匹配。实际 prompt/output token 为346/29、411/19，耗时130.3秒，两次模型请求，无重试或强制清理。证据 `/workspace/shared/pi-tools-validation/real-qwen35.json`。

## 证据身份边界

可提交脱敏摘要见 [PI 开发证据](../../examples/pi-desktop-tools/verification/2026-10-10-host.json)。模型/模板均有固定hash，实际worker握手验证协议5；真实推理时工作区尚未提交。最后聚合回归在模型运行期间重新链接开发EXE，因此事后记录的二进制hash不冒充启动时hash。两次构建之间生产源码没有改变，仅新增HTTP回归测试、独立桌面lock同步和文档/验收记录；这是开发闭环验证，不是精确发行二进制验收。Windows发布门槛须在最终冻结提交上重新执行。

## 完成范围与独立待验

上述 Linux 开发验证已完成，最终全 workspace 聚合回归689通过/11既有ignored、strict Clippy/fmt退出0。固定模型与真实客户端库的两轮工具链路通过，不意味着全部模型、工具或应用版本均通过。两个模型顺序加载；仅测试固定内存查表，没有执行任何用户文件、命令或外部业务操作。

Windows 完整交叉门槛、原生 Windows CI、安装包、用户 Win10/i5 与 PI Electron GUI 均未在本轮执行。按项目规则，推送触发 CI 前仍须完成 Windows 交叉构建；本轮不借用此前 9b18f41 的二进制验证。

## 用户授权后的 Windows 构建跟进

2026-10-10 用户确认按 Windows 交叉构建→推送 codex/dev→GitHub 原生构建顺序继续。新增原生 CI 显式运行 `llama-adapter --test tool_model -- --ignored`，使用既有固定 Qwen3-0.6B 基准文件，不额外下载或执行系统工具。失败保留 windows-real-tools.log；现有原生与安装门槛不减少。

前端从锁文件 `npm ci --ignore-scripts` 重建，1006 测试及 lint/typecheck/生产构建通过，保留 Vite 大 chunk 提醒。actionlint 1.7.12 校验修改后的 workflow 通过，严格 Python 全量仍367通过/5skip。独立交叉工具链正在恢复，以下步骤未执行完前不推送。源码/Windows实际结果将在后续记录补齐，不能将本节准备检查视为交叉门槛已通过。


### Windows 容量取整问题修复

推送前只读审查并以已恢复的 MSVC14.44 原始 xstring 头核对：std::string reserve(65536) 实际容量为65551，先前把主体长度上限误用为精确容量上限，Windows工具生成会直接失败。修复保留合法64KiB主体上限，单列每串32字节取整/SSO/终止符余量并验证实际capacity+1；raw上限65568、normalized最多49串合计67104、容器/结构体4096。均落在现有384KiB reservation中，不关闭容量检查或扩大公开输出上限。发布前释放pending，归一化结构仅保留真正发布的content/calls。

新增无模型 CTest 覆盖真实标准库满容量、多调用取整、超限、恶意大capacity和未发布字段释放。工具真实模型测试改为遵循NEXA_TEST_THREADS或min(4,available)，记录实际线程与oversubscription，Rust test-threads不再被误当推理线程预算。最终验证结果见随后的交叉记录。

容量修复后 native 构建、CTest 6/6、Qwen3-0.6B 两轮及线程配置2/2、adapter13单测/strict Clippy通过；新容量测试ASan/UBSan通过，LeakSanitizer因宿主ptrace限制未完成，不扩大为上游全量插桩或泄漏验证。原生CI显式构建清单同步新air-tool-output-test，并新增所有CTest可执行目标必须纳入清单的回归，严格Python373项中368通过/5skip。二次只读复审确认MSVC取整与输出副本预算闭合。
