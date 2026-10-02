# T07-B：MNN CPU 纵向链路契约与B1冻结接口

日期：2026-10-02。状态：**B1 native C ABI及Rust adapter已实现并完成本地Linux真实验证、Android arm64完整交叉链接；CI待验。B2 store/Executor/core接入仍为计划，Android真机及生产准入未完成**。MNN固定 `d407447ed56c4121a11ccbd266dc184ca1ead0c2`（3.6.1）。设计起点为Nexa `c1114ee`；当前接口以[实际C头文件](../native/mnn-shim/include/nexa_mnn.h)为权威，本文件不另行发布一份替代声明。实际命令/身份/限制分别见[原生B1验证](../native/mnn-shim/VERIFICATION.md)与[Rust B1验证](../mobile/runtime/VERIFICATION.md)，本地证据不等于CI或设备通过，不改变Windows协议。

依据：[T07计划](t07-android-mnn-plan.md)、[产品分期](android-app-parity.md)、[ADR0008](decisions/0008-android-mnn-engine-and-package.md)、[ADR0010](decisions/0010-model-artifact-and-conversion-provenance.md)。本轮只写本文件；其他工作区文档由主代理维护。

## 1. 推荐结论及已确认限制

已有 `mnn-adapter`（安全 Rust + 自有 C ABI）；后续B2计划新增 `mnn-executor`（专用线程 Executor）接现有core，另增独立 `mnn-model-store` 管理严格CPU文本包。不要把 `engine-host` 改成泛型 backend host，也不要让 Windows worker 链接 MNN。先实现真实 native load→prepare→generate→cancel→unload，再接 store/core；不先造空 trait 或 App 页面。

以下为未打补丁上游的限制；B1已用锁定私有补丁补齐请求采样/检查点，底层不可抢占与生产准入限制仍有效：

- `Llm::set_config` 只合并 JSON、更新模板/部分配置，不重建 `mSampler`；`Sampler` 构造时读参数并以 `random_device` 初始化 RNG。必须添加每请求重建/显式 seed 的受控补丁。
- `LlmContext::status` 是普通字段，`getContext()` 返回 const 指针；跨线程 const_cast 写状态、reset/destroy 都禁止。独立原子取消标志仅由控制线程写，原生 owner 在检查点读取。
- `generate(ids, 0)` 可做 prefill，但现有 chunk 循环没有取消检查；`generate(1)` 会把状态写成 MAX_TOKENS_FINISHED，不能直接反复调用它模拟安全 decode step。`forward(ids, is_prefill)` 当前忽略 bool 并 `updateContext(seq_len, 1)`，也不是可无脑组合的纯 prefill API。
- 原生 load 的 `Module::load`、单次 forward、Jinja 和 tokenizer 调用不能靠外层 atomic 立即中断。取消上限包含当前调用耗时；无返回就没有安全清理确认，不能报告已停止或再开一个实例。
- core 的 `ModelResolver::resolve` 已明文要求有界元数据查询，actor 内不能 hash 大权重；scheduler 同时要求 `ResolvedModel.validated=true`。候选可进入研究路径，不得为了打通链路伪造产品准入。

上述限制不阻止有界 CPU 实现；它们决定补丁、测试和产品准入的范围。CPU 1秒目标必须实测，不能在本草案里保证。

## 2. B2计划：最小Rust接缝及Windows兼容

现有 `Executor::start(ExecutorCommand, ExecutionEvents) -> Result<CancellationHandle, RuntimeError>`、`GenerationRequest`、`LoadOptions`、`Usage` 全部可复用。`MnnExecutor` 是可 Send 的 mailbox/control 外壳；真正 Model/Prepared/Tokenizer/KV/Sampler 留在线程内且 !Send/!Sync。

建议组合入口（Rust API，尚不存在）：

```rust
let registry: Arc<MnnRegistrySnapshot> = store.verified_snapshot(scope)?;
let resolver = MnnModelResolver::new(registry.clone());
let executor = MnnExecutor::new(registry, MnnCpuPolicy::v1())?;
// 之后依现有 Runtime::new 的真实参数接入，不另造调度器。
```

`MnnRegistrySnapshot` 是不可变 Rust 元数据：ModelId→包根/manifest身份/完整资产表/逻辑context/准入证据/实例内注册代际。resolver 与 executor 必须来自同一 snapshot；通过工厂绑定，不能拼接两个不同注册表。

首片**不修改** `runtime-types::ResolvedModel` 的 serde 形状：它没有把 path 定义为 GGUF 专属。MNN resolver 的 `path` 指向受控包的 `manifest.json`；MnnExecutor 从自己持有的注册表按 id 找完整条目，核对 path、代际及 context，不从任意外来 path 猜格式。manifest 是包资产描述，不直接传给 `Llm::createLLM`。Windows 仍使用现有文件 path，所有旧 JSON 字段/未知字段拒绝规则及 IPC v1 字节形状保持不变。

这不是跨进程 MNN DTO：MnnExecutor 是进程内固定平台组合，Windows process-host/worker 从不接收该条目。禁止在旧 `ResolvedModel.path` 编码 JSON、backend URL 或分隔符；禁止给 `validated` 偷加“hash正确”的含义。若将来需在 IPC/SDK 传递 format/backend/profile，新增明确 `ResolvedModelV2`/wire version及显式V1 GGUF转换，不以 optional serde 字段假装旧 `deny_unknown_fields` 读端兼容。

准入分两层：

1. 包校验正确不等于生产支持。现有候选五文件 hash 可完成真实 adapter/Executor 研究测试；直接调用 Executor 不经过产品 Runtime 的 validated gate。
2. **主代理已审定**：core 集成测试使用仅测试目标编译的 `ResearchCpuEvidence` composition，严格绑定实际 Linux artifact_digest、engine commit、patch identity、policy及真实通过证据，validated只覆盖这一研究域。不得做成生产Cargo feature/环境变量/运行开关，不向持久manifest写validated，不宣称Android支持；仅测试目标提供工厂，生产库不能链接该准入路径。生产 resolver 只依据受信设备/引擎/补丁/配置矩阵返回 true；第一片无 Android 证据时必须拒绝产品加载。

## 3. B2计划：多文件包V1与引用闭包

新命名空间 `MnnPackageManifestV1`，`schema_version=1`、`format=mnn_package`；不是给已有 GGUF manifest v1 加字段。单独 store/目录/解析入口，不修改 Windows `model-store::{ModelManifest,ModelStore}`。Rust 严格拒绝未知执行字段及重复 JSON key；数值必须有限且范围校验。

最小字段职责：

- id、display_name、format、schema_version；architecture、context_limit/default_context（来自已核验资产并受准入范围限缩）
- artifact_source：publisher、repository/source_uri、固定 revision、license；provenance_kind=`preconverted` 或 `self_exported`
- conversion_provenance：预转换的 original_revision/exporter_commit/arguments 可以显式 unknown；自行导出必须完整。unknown 不阻断运行输入复现，不授予转换复现
- files：按 UTF-8 相对路径排序的唯一 `{path,size_bytes,sha256,role}`；入口 config、metadata、graph、external_weight、tokenizer 的角色和引用明确
- template：来源文件、UTF-8文本 SHA256、固定上下文 `enable_thinking=false` 的策略ID与hash；tokenizer/EOS配置身份包含在资产/metadata hash内
- variants：首片只允许一个 `cpu-text-qwen3-v1` 变体，列出完整文件集合、受控运行策略版本；没有 fallback 变体/auto backend
- artifact_digest：对独立的规范化 identity payload 计算 SHA256（UTF-8、无重复 key、键排序、整数十进制、无浮点、无多余空白；文件表先按路径排序）。不把 artifact_digest 自身、准入证据、私有安装路径、缓存或 display_name 纳入摘要；payload 包含来源固定revision、全部文件hash/size/role、模板策略与变体。更改运行内容或策略必改身份
- validation 不是来源 manifest 自证；受信证据注册表单独按 artifact_digest + MNN/patch/build/policy/device 关联

首片只接纳 [候选锁](../scripts/android_mnn/candidate-model.json) 的 Qwen3-0.6B 五文件（revision `34dfccda1187ded6e07ea06426da576b0b793c6b`），不是泛化 MNN 导入器。检查 config/llm_config 实际白名单、`tie_embeddings=[275780066,431362530,19447808,8,64]` 和所有偏移/长度不溢出且在权重内；该模型 embedding 共享 weight，不凭空要求额外 embedding 文件。graph hash固定，因此未知内部外置引用不能从另一张图混入；通用图资产闭包扫描留后续，不宣称已支持。

导入：调用者显式提供候选本地目录→私有 staging（普通文件、无 symlink/hardlink复用、无设备/FIFO/路径穿越/绝对引用/大小写冲突）→逐文件有界流拷贝并hash→解析/闭包验证→fsync/原子发布不可变 generation。拒绝多余可执行资产和未声明 context/辅助图。只读 README/LICENSE 等随包保存时同样列明其 hash/用途，不交给原生扫描。

模型文件可能经原生按路径重开：不能声称保留 Rust FD 已消除 TOCTOU。采用 App 私有 copy（不零复制外部目录）、受控目录权限、publish后库内禁止改写、load期间 lease禁止更新/删除，load owner 中再按固定表校验；hash过程每块检查cancel。防护针对外部来源及正常App并发，不承诺抵御同UID恶意进程。测试故意换路径/增context应失败；若不具备该独占存储保证，禁止生产使用该方案。

原生运行配置由已验证元数据白名单重新构造在独立私有 work目录，不原样转交源 config。MNN的 `LlmConfig(path)` 会再 merge `llm_config`，故白名单校验必须覆盖合并结果并在 load前核验 `dump_config`，load后再验证并显式应用固定Jinja上下文。B1私有补丁在Nexa受控hooks存在时禁止context内容自动合并；B2还必须校验完整闭包，不得依靠冷僻的缺省文件名排除旁路。固定 CPU/high/low、async=false、reuse_kv=false、prompt/prefix cache=false、speculative_type空、全部 mmap=false、无visual/audio/talker。未来开放mmap须独立文件/缓存生命周期测试。

## 4. 已冻结自有C ABI V1及所有权

权威定义为 [native/mnn-shim/include/nexa_mnn.h](../native/mnn-shim/include/nexa_mnn.h)，SHA256 `40d1a79df99f5200d92e689baf791d5105d2da71d521fcd885a2807f0b916b8e`。前缀 `nexa_mnn_v1_*`，不复用 `air_*`；固定宽度字段、bytes指针/长度、struct_size/ABI=1、保留位和错误码均按该头文件，不再复制整组声明。Linux C11/Android C对象及Rust size/offset对照已验证；头文件变化必须同步hash、布局检查及artifact gate。

生成接口关键签名已冻结为独立的本次text与progress回调：

```c
int32_t nexa_mnn_v1_generate(
    nexa_mnn_v1_prepared *, nexa_mnn_v1_cancel *,
    nexa_mnn_v1_text, void *text_user,
    nexa_mnn_v1_progress, void *progress_user,
    nexa_mnn_v1_result *, nexa_mnn_v1_error *);
```

- load_options包含runtime_config_path、artifact/policy SHA、期望upstream/patch身份、logical_context/threads/prefill_chunk，以及仅load调用有效的progress/user。B1接受可信研究配置，校验artifact字符串不意味着已经对磁盘包做完整hash/lease；此责任属于B2
- request包含messages/stops、max_tokens/temperature/top_p/seed，以及**仅prepare调用有效**的progress/user；不会把其函数指针/user保存到Prepared后供generate继续使用
- prepared_info仅含prompt_tokens、resolved_seed及ABI/保留字段；result含prompt_tokens、completion_tokens、stop_reason、resolved_seed，没有拟议的load/prepare generation字段或阶段耗时字段。耗时由调用方数字诊断另记，不伪造未知值
- text callback接收 `nexa_mnn_v1_bytes`，每次1..4096有效UTF-8 bytes；0接受、1取消、2失败，其他值拒绝。回调1不是正常用户stop；stop字符串匹配由shim处理。progress是owner-thread数字回调，phase/count只表示检查点，不承诺延迟
- 每个回调及user只借用到对应C调用返回；消息/stop字节在prepare期间复制，Prepared留存预算所用token vector、stop副本、max_tokens/seed及model指针，不保留Rust输入字符串。callback不得重入model API、无限阻塞、保留text指针或抛异常

当前Model保存owner thread、Llm、logical context及busy/prepared/faulted状态；Prepared指向同一model且有consumed位。当前没有load_generation/prepare_generation字段，不虚称已实现代际校验；B2注册快照的代际绑定仍属计划。一个Model同时最多一个Prepared，不能在其存活时prepare另一个请求或destroy Model；C端检查错误线程、busy、重复generate，已free指针再用仍属调用者违约。

C generate在通过参数/ABI/线程/busy检查且开始owner操作后消耗Prepared，包括随后取消；这些前置检查失败不消耗。即便已生成，C调用者仍须destroy Prepared才能再次prepare/model_destroy。Rust `Prepared<'model>`持有Model可变借用，`generate(self, cancel, text, progress)`按值消费并尝试清理；Model/Prepared为!Send/!Sync。显式close幂等，失败保留所有权；Drop只做一次owner清理尝试，若回调内Drop另一句柄触发不可重入拒绝，保守泄漏而非强行释放，必须把句柄留到callback返回后close，不能宣称无泄漏。

cancel对象独立atomic<bool>，Rust `Cancellation`以Arc维护生命周期且不提供reset；只有cancel_request可并发，destroy须等所有操作结束/引用释放。不能给Model加unsafe Send/Sync绕过owner约束。partial load失败由同线程RAII清理，load/prepare失败*out清空。

C边界捕获异常并仅返回固定私密错误文本（error可空，错误头不合法不写）；Rust text/progress callback catch_unwind并映射明确失败。宿主panic hook仍会执行，panic=abort不能恢复；不安装全局panic hook掩盖它。SIGSEGV/abort/上游断言同样不是异常隔离能力，Android仍是App进程故障，不能套用Windows worker恢复结论。

## 5. 已实现上游私有补丁：owner-only请求扩展

现有补丁集由 [lock.json](../native/mnn-patches/lock.json)锁定，共3个patch、13个修改文件：`0001-nexa-cpu-request-v1.patch`、`0002-nexa-private-logging-v1.patch`、`0003-nexa-compiled-sinks-v1.patch`。只应用到独立副本，原版MNN checkout保持干净；第一补丁覆盖llm.hpp/llm.cpp/sampler.hpp/AR generation四文件，后两补丁覆盖静默日志及实际编译sink。不得再把早期“四文件/至少八文件”候选清单当成实际完整补丁集。不引入跨线程普通MNN字段或改写数值kernel；CPU证据不授予GPU/NPU支持。

受控C++扩展摘要（精确源码以请求patch为准，包含nexaCheckpointV1）：

```cpp
struct NexaRequestHooksV1 {
    void* user;
    bool (*cancelled)(void*) noexcept;
    void (*progress)(void*, uint32_t phase, uint32_t count) noexcept;
    // accepted token已记入output_tokens，terminal来自单次is_stop判定。
    // 返回0继续、1正常stop、2取消、3错误；仅owner线程调用。
    int (*token)(void*, int token_id, bool terminal) noexcept;
};
void Llm::nexaSetHooksV1(const NexaRequestHooksV1* hooks);
bool Llm::nexaCheckpointV1(uint32_t phase, uint32_t count);
bool Llm::nexaResetSamplerV1(float temperature, float top_p, uint32_t resolved_seed);
void Sampler::nexaSeedV1(uint32_t seed); // mRng.seed(seed)
```

hooks存副本，user仅操作期间有效；C++ token hook是shim内部owner操作，允许按既定顺序调用tokenizer_decode；下游Rust text callback不允许重入任何model操作。shim RAII在load/prepare/generate返回/异常时清除。native只调用cancelled，回调读取独立atomic，不借普通context给其他线程。progress仅数字、不得阻塞；真实取消测试据此证明到达指定阶段，不能用固定sleep猜mid-prefill。

`nexaResetSamplerV1` 仅owner空闲期使用：创建全新空LlmConfig，只显式设置temperature、topP及 `sampler_type=greedy`（temp=0）或 `topP`（temp>0）；上游键是 `topP`，不是API的 `top_p`。不继承旧penalty、logit_bias、banned_tokens和mixed/topK配置，构造全新Sampler，随后显式seed；不要只merge请求JSON保留旧状态。随机sentinel `u32::MAX` 由shim在每请求获取新熵并传实际seed，获取失败返回错误；固定seed包括0直接应用。临时sampler配置不得改模板/backend/文件路径。

prepare流程：验证请求→检查cancel→`reset()`→`generate_init(nullptr, "")`→采样重建→检查cancel→`apply_chat_template(ChatMessages)`→检查cancel→`tokenizer_encode(rendered)`→检查cancel→64位安全预算 `tokens.len()+max_tokens<=logical_context`→创建prepared并返回prepared_info；未来B2才发送ExecutorEvent::Prepared。限制原始输入1MiB，渲染结果另设固定上限（第一片1MiB，超出报错不截断）；设置后复核固定Jinja/nonthinking。native模板必须真实执行，独立golden只验证，不替换native输入。

生成直接消费prepared tokens；`generate_init`不会再被response隐式重做，不调用messages/string response，不重复套模板。B1使用显式prefill_chunk并关闭 `chunk_limits`；当前policy限制context≤2048、threads=1..2、chunk=1..128，本地真实样本为2线程/chunk32。B2不得将core默认batch_size=512直接照搬，应由移动composition显式提供合法配置，禁止静默夹限。不把公开 `forward` 当作替代路径。

补丁插入点和不变量：

1. load：shim在createLLM前后先检查cancel（构造函数也会读配置，尚不能装hooks）；load进入/必需文件检查后、initRuntime后、tokenizer后、DiskEmbedding后、Module::load后、clone/strategy load后、写RUNNING前检查。命中则返回false，由owner销毁partial model；不能先发Loaded再检查cancel
2. prepare：Jinja/tokenizer前后检查由shim完成；当前阶段不可中断耗时要单独报告
3. prefill：`generate(vector,int)`每个embedding/chunk前后，以及`generate(VARP,int)`中forwardVec成功且updateContext后检查。cancel必须break/return整条调用链，不能仅提前返回内层后外层继续下一chunk。发布成功chunk进度后读cancel，禁止进入decode
4. AR：while顶、采样前、已计token后的回调后、forward前后检查。先sample→history/output追加→updateContext(0,1)→一次is_stop→token callback；special stop也回调以统计usage但不解码。有hooks时不写ostream、不累计generate_str；无hooks保留上游旧行为
5. C++内部token hook返回正常stop（与对外text callback返回1取消不同）时owner写NORMAL_FINISHED；取消写USER_CANCEL，失败写INTERNAL_ERROR；不得由结尾 `len>=max_token` 覆盖已有终止原因。仅达到输出预算且尚无stop/error/cancel才MAX_TOKENS_FINISHED
6. 最后一个预算token、special stop、用户stop和callback拒绝后不再为“下一token”跑forward。当前上游AR会对末token多跑一步forward；受控分支消除此额外计算，并据独立token golden确认没有漏token/错usage

检查点不能从kernel中途longjmp/throw强退，不能在cancel线程调用set_config/reset。恢复是在owner完成本次调用及必要清理后；若一般推理错误使KV状态可信性不明，销毁模型进入faulted/需显式重新load。正常取消完成后清理KV/历史并重建下请求sampler，真实测试证明恢复后才保留loaded实例。

### 5.1 精确补丁身份门禁

当前patch-set SHA256为 `43cc33146e2036ff452bd02d5ec352bb099d143ed4a4cdeb6ff55335987f9ce0`，policy SHA256为 `ea06621b78e67e58f97f98951b26db0a8a893ded4112da3e5a762b98566fa328`。每个patch及13文件before/after SHA以[lock.json](../native/mnn-patches/lock.json)为权威，不在本文维护另一份易漂移表。本地独立全新副本精确重放/postimage核验已通过，build_info嵌入upstream/patch/policy/silent身份，Rust artifact gate另核对header、全部archive、compiler/target/NDK/API等；CI尚待实际验证。

必须继续执行：精确基线与补丁顺序、无fuzz应用、构建前后postimage检查，不接受仅tag、补丁名或dirty=true。任何补丁/头文件/构建输入变化都更新lock/artifact身份和回归证据；允许列表中的工具链profile不是该profile的通过证据。

### 5.2 生产默认日志：编译期抑制，额外补丁范围

未打补丁上游不是安全日志：`include/MNN/MNNDefine.h:22–40` 在开启LOGCAT时走Android日志，关闭时走printf；`llm.cpp:62–87` 的 `LLM_LOG_TO_STRING` 先向原sink输出再无限追加 `mContext->log_buffer`。`Llm::getLog()` 只是取出缓冲，无法阻止泄漏。`llmconfig.hpp:67–68,96` 的std::cerr和 `tokenizer.cpp:81,91` 的printf直接包含文件路径，绕过MNN日志宏；`tokenizer/jinja.hpp:41–45` 的JINJA_DEBUG也有独立stderr路径。

**B1已实现并完成本地锁定配置审计/canary，CI及Android日志待验的机制**：固定编译期 `NEXA_MNN_SILENT_LOGS=1`，在MNNDefine.h最高优先级分支将 `MNN_PRINT(...)` / `MNN_ERROR(...)` 定义为 `do {} while(0)`，不格式化、不求值日志参数、不分配、不留原日志副本。该分支作用于Nexa私有MNN构建的所有MNN/Express/llm及shim目标与头文件消费者，不能只给llm target设置。上游日志表达式有副作用的点必须审计，不得因去掉参数求值改变必要计算。保留原宏行为供非Nexa构建，禁止通过运行时开关恢复私有构建日志。

第二patch抑制MNNDefine日志宏及llmconfig/tokenizer/unicode直接输出；第三patch继续处理ConvolutionCommon、embedding、omni、dflash、eagle实际编译sink，全部身份见lock。Linux314编译单元/242依赖头、Android445/220的本地审计均零未分类直接sink；头文件的默认cout参数、literal #if 0和关闭的调试分支按源码hash分类，不把所有出现printf的文本都误判为执行输出。审计/canary限定当前源码与CPU文本profile，不是所有系统库可达性的形式化证明。

Nexa构建同时禁用 `LLM_LOG_TO_STRING`、`JINJA_DEBUG`、`DUMP_PROFILE_INFO`、张量/内存/文件dump调试；校验compile_commands中所有关联target的有效定义，发现冲突必须构建失败，不靠“默认OFF”。`MNN_USE_LOGCAT=OFF`只是辅助链接配置，**不能代替静默补丁**。禁止在App里dup2关闭stdout/stderr、替换进程级iostream rdbuf、宏劫持所有printf/snprintf、LD_PRELOAD或安装全局可变callback；这些会影响App其他线程/库，且snprintf可能参与真实token处理。

错误与诊断另由shim产生：固定白名单错误码/阶段/计数/耗时，错误文本最多512 UTF-8 bytes且只从固定字面量选择；不把上游 `what()`、getLog、模板、prompt、生成文本、token片或完整路径转发、缓存、写默认日志。若保留诊断队列，采用现有最多16份数字报告try_send，满了丢诊断不阻塞推理；无任意format string日志hook。这样没有需加锁的全局日志callback，也没有“先收集完整敏感日志再正则脱敏”的泄漏窗口。

B1已完成锁定构建的本地编译/可达闭包审计；以后构建和源码变更仍须：根据实际compile_commands逐个检查直接printf/fprintf/iostream/Android log、文件dump和日志宏重定义；`omni.cpp`、speculative dflash/eagle等虽不属首片功能，可能仍被上游CMake编译，须证明受控config无法到达相关动态输出分支或在私有构建补丁中抑制。`source/core/ConvolutionCommon.cpp`的固定printf、其他cpu/express诊断也须显式列入已审计清单，不能以扫描一个llm.cpp宣称全覆盖。新增任何必要补丁文件立即扩展identity lock，不擅自保持早期候选文件数。静态扫描允许区分字符串内存格式化与真正输出，但例外须有精确源码hash、调用边界及理由。

本地已通过8项配置/文件负例及真实中英/多轮输出marker、取消/callback失败恢复矩阵；宿主并发日志保留。详细分类与限制见原生VERIFICATION。持续日志验证门禁：成功中英/多轮、缺失/损坏tokenizer/graph/config、模板解析失败、load取消、prefill/decode/背压错误均注入不同prompt/output/path canary；Linux测试进程捕获stdout/stderr，Android测试收集本App日志验证。独立并发测试让宿主线程写自己的测试日志，确认Nexa没有重定向或吞掉宿主输出；上游私有日志必须不含任何canary，正常推理也不向默认sink输出正文。既检查敏感marker也核验预期固定输出集合，不能以未生成到canary当作通过。运行日志/元数据输入使用纯合成内容；Android日志未测仍列待验。

该机制不改变OS崩溃转储的权限或保证原生abort绝不产生系统诊断；它覆盖Nexa可控的默认应用日志。无法解释的动态输出、无法证明生效的目标宏或剩余可达直接sink，都阻止生产日志门禁通过。

## 6. 输出、stop、usage与背压

token回调由shim owner调用 `tokenizer_decode` 取得原始byte piece，special stop不解码。piece不是UTF-8字符边界；绝不对每token `from_utf8_lossy`，不丢字节、不剥think标签。B1已有MNN私有stream_buffer及golden，包含“stop前残缺UTF-8不能吞掉”的修复，不改Windows copy；纯stream ASan/UBSan已通过，不能扩大成完整MNN sanitizer/泄漏证明。

- 单token piece上限1MiB；stop最多4条、各1..128 UTF-8 bytes，pending保留可能跨token匹配的最长前缀和最多3个未完成UTF-8字节，当前piece处理完就流出，不累积整段回答
- 检测最早stop byte位置，去除stop及其后文本；重叠/跨token/同piece多个stop、中文/emoji切片测试；相同位置无需暴露哪条stop先命中
- 正常EOS/length时刷新完整UTF-8；最终残缺或非法UTF-8返回NativeFailure，不替换。取消/断连后不强发pending尾片，不泄漏可能的stop前缀
- B2计划将callback输出≤4KiB UTF-8片直接 `ExecutionEvents::text_delta` 使用现有256KiB单账本和10秒无消费进展超时；返回false使native停止，不能再加无界channel或第二份text queue
- 回调阻塞期间core取消/断连/slow-consumer应唤醒Output等待，owner返回后即检查atomic；未确认唤醒语义就不能称背压取消已完成
- Usage.prompt_tokens=完整prepared vector长度；completion_tokens=已采样接受token数量，包含special EOS和命中stop的token（即便无文本发出）；主代理已复核与现有llama shim的increment-before-is_eog语义一致，不等于上游gen_seq_len/ostream字数；失败/取消也返回已知准确计数
- 每个operation只发一次executor终结事件，core拥有唯一用户终态。已采样但取消后未显示的token仍计usage；emit完成不等于客户端已经消费。Prepared失败usage.prompt=0，Prepared成功后prefill失败保留完整prompt计数

## 7. B2计划：执行器生命周期与有限内存

`MnnExecutor::start` 分配一次性cancel并try_send到容量1 mailbox及时返回；Load/Generate/Unload同一owner串行。操作ID沿用core防旧事件误配。Load时hash/创建运行配置/native加载在owner执行，不堵actor；generation负载不能使cancel排队。unload成功事件只在Prepared、Llm、其Module/KV/Runtime依正确顺序释放后发送。

close只在core确认无活跃操作及卸载后关闭mailbox并join owner；若owner异常退出无法证明native cleanup，不发送伪Unloaded。Drop只请求cancel、关发送端，不能join卡死kernel或异线程析构model；owner自己持有package lease直至退出。无返回时保持stopping/不可用，不能以超时释放句柄或新建并行实例。close/Drop和idle unload竞态单列测试。

限制：输入/渲染/token vector、piece、stop pending、core output各有上限；MNN内部history/output vectors受logical context/max_tokens约束，controlled hook关闭generate_str。模型、KV、算子临时内存仍由原生图决定，不把256KiB输出账本宣传为总内存限额。第一片仅已测context≤2048、max_tokens≤context预算；提到131072只是公共参数解析范围，不是该模型准入能力。

## 8. 最小实施切片、单写者与测试门禁

### 8.1 构建边界决定：独立移动Rust workspace

采用 `mobile/runtime/Cargo.toml` 的独立 `[workspace]`（resolver=3）与 `mobile/runtime/Cargo.lock`，当前唯一成员为 `crates/mnn-adapter`；`crates/mnn-executor`、`crates/mnn-model-store`为B2计划成员；这些名字不是App/SDK脚手架。根workspace已有显式members列表，首片不向其加入MNN成员，不改根Cargo.lock，也不把MNN作为Windows crate依赖。独立workspace清单本身应阻断父workspace归属搜索；以Cargo metadata实际验证，不凭目录名判断隔离。

B2计划共享 `runtime-core`/`runtime-types`，用workspace根的路径依赖 `../../crates/runtime-core` / `../../crates/runtime-types`；它们继续属于原根workspace，自己的 `version.workspace`/serde等继承原根定义。这种“独立workspace依赖另一workspace的path crate”已有桌面壳参考：`apps/desktop/src-tauri`独立workspace依赖根desktop-bridge/runtime-api。不能复制core/types源码来绕开继承，不移动原crate，不把整个rootworkspace变为dependency。移动侧新依赖单独锁定并核对共同serde/uuid等版本；锁文件范围变化不升级Windows依赖。

不选“根成员+默认关闭native feature”：根CI对workspace全测/clippy容易触发feature统一、build.rs或all-features，也容易为了Windows编译塞入没有真实执行能力的fake executor。移动native构建是显式独立命令；需要MNN的target若缺锁定native输入就明确失败，无静默fallback、dummy symbols或fake文本。纯包解析测试可在不编译adapter的指定package里运行，不改变生产executor身份。

B1已交付独立 `native/mnn-shim/CMakeLists.txt`、[include/nexa_mnn.h](../native/mnn-shim/include/nexa_mnn.h)、native验收器和Rust adapter。具体ABI字段/布局、独立per-call progress/user和回调所有权以实际头为准；Linux C11运行、Android C对象及Rust布局断言通过。后续不得重新引入早期拟议 `nexa_mnn_v1.h` 或省略参数的文档签名。

现有 `mnn-adapter/build.rs` 只消费显式提供且核对target/toolchain/ABI/patch/build identity的本地native产物；不自动联网下载MNN/模型/NDK。Linux开发和Android arm64分别用独立产物目录/target目录，不能宿主库混到Android链接。Android完整链接的C++ runtime/必要系统库由固定native构建identity提供，编译/链接失败明确上报；Windows direct-build移动adapter返回清晰“不在本次支持范围”，正常Windows根CI不会走到它。

本地已验证独立cargo metadata/workspace_root、根Cargo.toml/Cargo.lock无变化、两个lock共享依赖版本一致；Rust6单测+4 compile-fail文档测试、显式真实模型套件、Linux/Android clippy与fmt、Linux8项/Android10项artifact拒绝通过。真实测试默认ignored不作运行证据，必须经显式runner执行。

Android已完成两个Rust测试ELF和build_identity ELF的Rust→shim→MNN→静态C++完整链接，非仅cargo check。NDK r30/API28/arm64；最终LOAD及GNU_RELRO末端均16KiB对齐，所需系统库闭包见Rust验证记录。早期只有LOAD对齐的结果不能冒充最终完整页门禁；libatomic纯注释占位被正确拒绝后改用真实clang builtins archive，未放宽archive验证。Linux真实运行、Android完整链接及CI/设备运行分别记证。

后续仍须保持根Windows依赖图无MNN及原有回归、移动lock/独立target-dir/target身份门禁；B2生产与仅测试ResearchCpuEvidence composition须验证依赖图/符号和负例，不使用产品可启用的feature开关。

### B1：原生CPU链及Rust adapter（已本地验证，CI待验）

实际交付按目录单写者分工：native owner维护 `native/mnn-shim/`、`native/mnn-patches/`，Rust owner维护 `mobile/runtime/`；root整合文档/CI。并非一名实现者同时占有全部目录。根Windows workspace/Cargo.lock、共享MNN原版、llama-shim和engine-host未因本片改写。真实load→prepare→generate/取消/释放已通过本地原生及Rust测试，完整证据和未覆盖项以两份VERIFICATION为准。

已覆盖的本地门禁及以后回归要求：C ABI版本/sizeof/null/枚举/溢出、同线程与prepared一次性/释放顺序、ASan/UBSan纯shim/stream测试；真实候选中英/空内容/system/多轮、nonthinking渲染及token序列、正好预算/超1、EOS/max1/用户stop、A→B→A采样隔离。合成logits测试topP/temperature/greedy/seed；相同seed真实同环境可复现，但不能以不同seed必须不同文本作为唯一断言。用hooks阶段屏障触发load/prepare/prefill/decode取消，各自确认安全返回并再生成，记录响应时间而非只看cancel写入。

### B2：受控store + MnnExecutor/core研究组合

所有权：另一名实现者可在B1头文件冻结后独占 `mobile/runtime/crates/mnn-model-store/`、`mobile/runtime/crates/mnn-executor/`；不得同时改B1接口。纯解析/存储测试可与B1并行，真实接缝必须依B1通过。root维护已审定的仅测试目标ResearchCpuEvidence门禁及规范同步。

门禁：五文件缺失/hash/路径穿越/symlink/未声明context/metadata覆写/embedding溢出、staging中断/原子发布/空间失败、load lease与删除竞态、snapshot代际不匹配；真实执行器load→prepared→stream→cancel→unload、慢消费者256KiB/10秒/断连与取消恢复、请求终态一次、load超时仍等安全返回、关闭/idle竞态。候选合法hash但无产品证据必须被生产resolver拒绝。没有App生命周期代码，不把模拟后台cancel当真机验收。

### B3：设备证据（B1交叉构建已有，设备待验）

B1已以锁定NDK r30/API28/arm64构建完整Rust→shim→MNN闭包并验证LOAD/GNU_RELRO页对齐。后续继续核对needed/CRT与许可、build_info精确commit/patch/调用方研究资产身份；包资产完整闭包仍依赖B2。Linux CPU结果、Android链接成功、Android设备运行三层分报。

真机CPU执行上述真实场景、phase取消最大值/分位数、重复load/unload和内存趋势；没有设备继续保留“待验证”，不授予production validated、不宣称APK完成。OpenCL/QNN/Hexagon等后续新增profile不复用此验收结论。

每片相关Rust fmt/clippy/unit；涉及workspace或公共链接时重跑Windows原有真实链/序列化golden，旧GGUF managed/external schema及IPC fixtures原样读写，Windows binary dependency检查无MNN。Android支持不得由Windows回归替代；反之亦然。

## 9. 源码耦合风险与审查决定

- chunk路径更新 `prompt_len`/KV计数的时机、末token额外forward和 `is_stop`修改status都容易误读；补丁必须带exact upstream黄金token对照及usage断言
- LlmConfig的二次merge、load读取context、默认辅助文件路径是闭包旁路；限制固定候选配置优于先开放任意MNN目录
- `Sampler::topP`算法和C++随机分布不保证不同标准库/硬件逐token一致；记录工具链，不承诺与llama输出一致
- callback在owner阻塞是可取消背压，不能持有store/actor互斥锁，不能在回调里调用Runtime同步方法造成死锁
- 无真正可抢占kernel的API；若已测chunk/load调用超过CPU取消阈值，该配置验收失败，需要缩小chunk/限制输入或进一步专门上游工作，不用假确认掩盖
- 新包store分离带来少量重复hash/事务工具；先稳定实链再评估提取共享工具，避免本轮破坏Windows历史行为

主代理已审定：不改公共DTO、MNN独立注册快照、B1原生优先、仅测试目标ResearchCpuEvidence及EOS usage对齐。5.2编译期静默日志方案已接受为实现要求；实际已扩展为3patch/13文件并通过本地锁定构建审计/canary、token hook/终态与原版golden；CI与Android设备未替代。不可中断load区间的准入耗时仍需设备最大值/分位数，现有Linux检查点单轮样本不是1秒承诺。本文件不是已发布协议，未完成日志/安全门禁不得宣布生产完成。

## 10. 本轮文档同步检查

任务：将设计草案同步至B1实际冻结接口。只修改本文件，读取实际C header、native/Rust VERIFICATION、补丁lock及必要实现；复算header SHA与报告一致。修正独立generate progress/user、文本回调0/1/2语义、prepared_info/result实际字段、C/Rust消费/Drop失败边界、3patch/13文件日志闭包和独立workspace当前唯一成员；B2/生产store/Executor仍明确为计划。

本轮仅作链接存在、围栏配对和空白检查，不重新运行构建/真实模型测试，引用的通过结果来自B1验证记录；CI待验，无Android运行或生产准入结论。无stage/commit/push。T07-A历史仍见[验证记录](verification/2026-10-02-t07a-mnn-cpu-probe.md)。
