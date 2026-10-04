# ADR0015：开放模型加载与验证证据分离

2026-10-04 补充：[ADR0019](0019-model-onboarding-and-local-validation.md) 新增独立本机加载/基础生成记录，不改变本文历史 validated/validation、开放 loadable 或全面兼容性边界；本机短测结果不冒充原精确验收矩阵。

日期：2026-10-03。状态：用户方向已采纳，W02开放加载源码已冻结、本地回归通过，独立审查与精确WindowsCI待完成；不是已交付功能或广泛模型实测完成声明。

## 背景与用户目标

用户明确目标机内存为16GB，并希望支持很多模型，而非仅特定几个。Windows10 x64/i5-8400为既有首要目标；实际可用内存、其他进程占用与具体模型速度尚未测量。

原实现将精确Qwen3-0.6B矩阵同时作为历史验证证据和运行许可名单，导致没有实测记录的其他模型不能尝试加载。这不符合当前用户目标。1.7B/4B等候选应是基准样本，不是产品型号白名单。

## 决策

1. **开放候选加载**：不按模型名、系列、文件名或事先收录的hash决定能否尝试。符合本版本安全/结构/文本执行边界的单文件GGUF可作为候选，由锁定llama.cpp实际loader判断架构、张量和执行支持。
2. **验证与资格分离**：`validated`、`validation`与既有capabilities保留精确历史证据；新增独立`loadable`表示受控manifest满足候选条件，可进入加载流程；实际提交native前仍须通过store的文件完整性/结构准备，绝不是模型兼容、质量或内存保证。`available`反映候选资格及当前已观察到的文件/目录错误，不等于load必成功。
3. **hash继续保护身份**：全文件hash、metadata匹配、模板hash、路径/普通文件检查、TOCTOU防护和external Windows lease保持。取消“批准hash名单”不等于取消文件完整性校验或允许伪造validated证据。
4. **结构边界公开**：当前切片只接单文件GGUF v2/v3、已经实现结构校验的tensor layouts、非空原始嵌入模板。分片模型、未知结构、缺模板及不符合边界的资产明确拒绝；不让llama自动读取未经保护的邻接分片。普通/K量化之外的未实现layout不能靠文件名放行。
5. **文本模板边界**：使用模型自己的Jinja和vocab，禁止fallback模板、重写system/role或静默丢内容。要求生成prompt与实际assistant文本continuation保持一致、结束后缀符合vocab终止规则；不支持的思考/工具/输出framing明确报错。此检查不等于支持全部Jinja或模型输出格式。
6. **内存与上下文分开**：候选context按模型metadata与既有131072硬上限约束，真实模板tokens+输出仍不得超请求context。131072是协议/实现上限，不是16GB机器能运行的承诺；本切片没有新增Job RAM硬限，不宣传OOM完全隔离。
7. **界面与API诚实显示**：未实测候选可显示“可尝试加载”，不因validated=false自动禁用。实际load失败保留可读错误；旧服务没有loadable时要求匹配版本，不伪造候选资格。历史`admitted`标签只表示有精确证据，不再授予排他的模型许可。
8. **当前目录事务不变**：目录扫描仍为整批原子发布；一个不支持/损坏GGUF可使本轮登记整体失败，旧目录/索引保留。逐文件诊断与部分候选登记为后续切片，不谎称已经实现。
9. **工具能力独立**：开放文本加载不授予工具调用、结构化输出或harness agent可靠性。工具协议、模型质量和Windows真机分别测试；后续工具契约另行设计。

managed导入前、manifest与load统一单文件≤16GiB；external原16GiB限额保持。该文件读取/登记预算不是16GB RAM成功保证。metadata context小于默认2048时，当前默认登记仍失败；自动扫描改取min与逐文件诊断属于下一片，用户显式参数不会静默夹紧。

原始模板、metadata key与tensor name含NUL时拒绝，避免Rust/native的C-string身份截断；普通tokenizer metadata values含NUL不一概禁止。Engine初始化强制关闭common/Jinja日志并使用受控静态模板错误，最终隐私canary只证明被覆盖的成功/异常路径，不作绝对无泄漏承诺。

### 模板执行与资源限制

Jinja目前没有循环、操作数或中间分配预算；4MiB限制在render完成后才检查，构造期的模板能力探针同样没有内部执行预算。因此不能把输出大小上限称为模板执行时间或峰值内存上限。

Windows产品worker由父进程独立计时：加载使用load_timeout，prepare属于Generate的execution_timeout，默认各300秒；超时发取消，5秒宽限后可调用TerminateJobObject，再至多等待5秒确认并有界清理。取消标志不能保证中断正在执行的Jinja；未确认回收则fail-closed，不宣称已停止或另起worker。该机制避免父端无界等待，不构成Job RAM硬限制，超时前仍可能OOM或拖慢16GB系统；直接进程内使用adapter没有这层父进程强杀保护。

复杂模板的执行/分配预算属于后续待办，不扩入本切片；当前不作全部模板安全或全进程内存安全承诺。

## 版本与迁移边界

- manifest schema1保留历史字段与证据检查；不把旧未验证资产改标validated=true，不改旧报告
- ModelSummary拟新增loadable与可尝试上下文信息；内部ResolvedModel的validated门槛改为独立loadable
- 私有IPC从v1升为v2，shim行为身份从2升为3；公共协议仍1，C ABI布局保持v2。adapter的Engine::new读取实际build_info并严格拒绝旧/伪造archive，worker Hello也校验，打包/验收器身份同步；不得混用旧父进程、worker或AIR_NATIVE_DIR。版本变更本地已验，精确WindowsCI另验
- 最新已交付389eeef和文本验证基线35bfd85仍保留旧行为，不能因本ADR更新宣称其已开放模型；本切片精确提交/CI/新包另记

## 需要保留的验收

跨模型名/hash无白名单的候选测试、manifest伪造证据拒绝、结构越界/分片/未知layout拒绝、原始模板/文本终止、每次load完整性与TOCTOU、UI/API可尝试状态、上下文预算/资源错误及旧固定模型真实回归均须有证据。详细矩阵与实际进展见[W02记录](../verification/2026-10-03-windows-open-models.md)。

本决策替代当前主文档中“只有矩阵中的几个模型可加载”的限制，不取消精确可复现验证矩阵，也不把“开放候选”解释为“全部GGUF保证支持”。原ADR0003/0013/0014和历史报告中旧模型门槛仅用于追溯。
