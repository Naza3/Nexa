# T07–T08 Android MNN 执行计划

日期：2026-10-02。本文为下一阶段设计与验收计划；Android/MNN 代码、模型包、APK、CPU/GPU/NPU 和真机结果均**未实现、未验证**。本轮完成方向文档迁移，不修改源码，不改变 Windows 已有实现或将上游示例记为 Nexa 成果。

阅读入口：[架构](architecture.md)、[执行规格](../ai-runtime-v0.1-execution-spec.md)、[开发路线](roadmap.md)、[构建锁](build-lock.md)、[模型矩阵](model-matrix.md)。动态任务状态仍由[当前状态](../PROJECT_STATE.md)维护。

## 1. 已确认方向与实施边界

1. Android 主推理引擎采用 MNN，保留 Rust 通用类型、模型管理、调度和生命周期控制层。Windows 继续使用现有 llama.cpp/worker 链路；不先实现 Android llama.cpp，再迁移 MNN。
2. CPU、GPU、NPU 全部进入计划。工程顺序为 CPU 基线与安全契约 → 最小前台 APK → OpenCL → QNN v79/v81 → 直接 Hexagon 实验。纳入计划不等于所有后端同时发布，后端失败也不取消已通过的 CPU 基础路径。
3. 目标覆盖 Snapdragon 8 Elite 及后续代际，按实际 SoC/驱动/模型组合逐项准入；后续芯片不会仅因名称更新就自动标为支持。
4. 最小 APK 仅承载本地模型包导入、前台文本生成、流式显示、停止、状态和诊断。UI 沿用 Flutter + flutter_rust_bridge 的方向，精确版本在构建成功后锁定。不增加聊天业务、云账号、Telegram 接入、后台常驻服务或 NPU 性能承诺。
5. 本计划不授权下载 SDK、接受新许可、注册账号、安装驱动或改变系统设置。Android SDK/NDK、QAIRT/QNN、Hexagon SDK/Tools 等新增依赖先列出版本、来源、协议、用途和再分发范围，在需要接受新协议或获得新权限时另行取得批准。

### 1.1 与旧规范的迁移关系

[ADR0008](decisions/0008-android-mnn-engine-and-package.md)替代旧规范中“第一版只接 llama.cpp”“单文件 GGUF”“不接 MNN/QNN”及 Android 复用同一 adapter 的方向。架构、执行规格和路线已同步；通用调度、预算、取消、唯一终态、前后台及隐私要求继续保留。

本轮仅同步方向与验收边界：Windows `llama/GGUF` 与 Android `MNN/package` 分开，尚未实现源码迁移。T07-B实施前再冻结具体模型包schema、C ABI和公共DTO的兼容设计；本文中的候选字段不是已发布协议。旧Windows验收不能追溯为新公共类型或Android通过。

## 2. 当前源码与必须解除的耦合

以下为本轮只读检查确认的事实：

| 现有入口 | 实际约束 | 下一切片处理 |
| --- | --- | --- |
| [Executor](../crates/runtime-core/src/executor.rs) | core 已有 `Executor`、`ExecutionEvents`、独立 `CancellationHandle` | 复用唯一调度器；MNN 实现同一执行器契约，不复制队列/状态机 |
| [EngineHost](../crates/engine-host/src/lib.rs) / [依赖](../crates/engine-host/Cargo.toml) | 直接导入并依赖 `llama-adapter`，不是可直接承载 MNN 的通用宿主 | 新增实际工作的 `MnnExecutor`，由 mobile 组装；首轮不重构 Windows 原生链 |
| [ResolvedModel](../crates/runtime-types/src/scheduler.rs) | 以单个 `path` 等字段描述加载输入，没有 MNN 文件闭包/产物类型 | 引入有版本的模型资产描述；区分 GGUF 与 MNN package，不把目录假装成 GGUF 路径 |
| [ModelManifest](../crates/model-store/src/manifest.rs) | schema 1 使用 `relative_file`、单文件 hash、`gguf_file_type`、`validated_llama_commit` 和固定 GGUF 验证矩阵 | 新 schema/格式分支与迁移测试；旧 schema 1 可读且证据含义不变 |
| [模型存储](../crates/model-store/src/store.rs) / [外部目录契约](t06-model-directory-contract.md) | 当前 GGUF 导入、解析和 Windows 外部目录行为已有独立安全边界 | 保留 Windows 行为；Android 首轮采用包导入到 App 私有目录，不扩展为任意目录直读 |
| [llama build.rs](../crates/llama-adapter/build.rs) | 明确只允许 Linux 开发验证与 Windows MSVC，拒绝 Android | Android 新建 MNN 构建入口；不删除此保护来“修好 Android 编译” |
| [请求参数](../crates/runtime-types/src/lib.rs) | `temperature`、`top_p`、`seed`、stop、精确 usage 已有语义 | MNN 逐项实现/验证；不能只接受字段却忽略它们 |

建议新增 `mnn-adapter`、`native/mnn-shim`、移动组装入口；实际目录/命名与具体接口在实施切片冻结，本文不代表这些模块已存在。Android 依赖图不包含 `llama-adapter`、PC HTTP、IPC 或 process-host；Windows API 仍不得链接任一原生引擎。若公共 DTO 需要扩展，同步受影响编解码并回归 Windows，不能把 MNN variant 直接发给不认识它的旧 worker。

## 3. 版本研究基线与设备矩阵

### 3.1 版本状态

[MNN 3.6.1 发布页](https://github.com/alibaba/MNN/releases/tag/3.6.1)提供直接 Hexagon 后端和 CPU/OpenCL 等路线的研究依据。发布页对应[候选 commit `d407447ed56c4121a11ccbd266dc184ca1ead0c2`](https://github.com/alibaba/MNN/commit/d407447ed56c4121a11ccbd266dc184ca1ead0c2)。这是本轮查到的上游身份，**尚未成为 Nexa 构建锁**；不得用“3.6.1”标签或文档网站的版本页眉代替精确依赖身份。

T07-A 必须复核 tag→完整 commit、子模块/第三方依赖、许可证、导出器和运行时是否同源，选定可构建组合后记录：MNN commit、Nexa shim ABI、补丁集及 hash、编译选项、NDK/Clang/CMake/Ninja、Android API 级别、Rust target、JDK/Gradle/AGP、Flutter/bridge 版本。更换 commit 或模型导出器须重新验证，不自动跟随 master。GPU/NPU 可因必要修复选择后续精确版本，但作为独立升级记录，不能污染已固定的 CPU 对照。

### 3.2 公共参考设备

| 档位 | 公开参考 | 计划用途 | 当前结论 |
| --- | --- | --- | --- |
| 首测参考 | OnePlus 15；厂商规格列 Snapdragon 8 Elite Gen 5；Qualcomm 对应 SM8850 / SOC ID 87 / Hexagon v81 | Android CPU、OpenCL 与 QNN v81 主测档；直接 Hexagon v81 独立实验 | 仅参考型号，未测 |
| 兼容档 | Snapdragon 8 Elite / SM8750 / SOC ID 69 / Hexagon v79 | CPU/OpenCL 兼容回归、QNN v79；直接 Hexagon v79 研究对照 | 未测 |
| 后续代际 | 新 SoC、Hexagon 架构、系统/驱动组合 | 探测、工具链/产物适配后单列矩阵 | 不预授支持 |

型号依据：[OnePlus 15 官方规格](https://www.oneplus.com/us/15/specs)；SoC/Hexagon 映射依据：[Qualcomm QAIRT 支持表](https://docs.qualcomm.com/nav/home/QNN_general_overview.html?product=924033590759186372)。支持表说明 SDK 的目标范围，不证明任意手机固件、MNN 模型或 APK 已可运行。

实际诊断再记录 Android/API 版本、ABI、页大小、SoC、可用内存、GPU/OpenCL 版本、厂商驱动、可用 DSP/HTP 能力及 SDK/runtime 身份。本文不假定当前 Android 版本、RAM 容量或 NPU 驱动可用。仓库仅保留公开型号与脱敏能力档，不写设备序列号、账号、截图、完整路径或私有手机测试细节。

## 4. 原生契约先于功能页面

### 4.1 C ABI 与资源归属

MNN C++ API 留在自有 shim 内；Rust 只接收版本化固定宽度字段、不透明句柄和 UTF-8 指针/长度。接口应覆盖 build info、加载、模板/分词准备、生成、卸载、独立取消及错误缓冲释放。名称在实施时冻结，不与既有 `air_*` llama 符号发生链接冲突。

- MNN Llm、Tokenizer、sampler、KV、模型/执行图及 prepared 句柄由同一推理线程创建、使用、释放；只允许独立原子取消标志跨线程
- `Executor::start` 及时返回；取消不经正在生成的工作队列，关闭/卸载只在资源安全释放后确认
- C++ 异常由 shim 处理，Rust panic 不穿越 FFI；核对上游无异常构建/断言终止路径，不能把 catch 声明为能隔离原生崩溃
- callback 数据仅借用；UTF-8、stop 匹配和本次 token 暂存分别有上限，不用积累整段输出后一次性通知的 demo 冒充流式
- 继续使用 core 的单一输出账本：每请求 256 KiB、单 delta ≤4 KiB、10 秒无消费进展触发取消；另外量测 MNN 内部文本/token 容器、模型、KV 和计算缓存，不把该账本宣传为全部 native 内存上限
- 关闭跨请求 KV/prompt/prefix 缓存、投机解码、多模态及网络资源功能；不把上游默认功能自动暴露为 Nexa 能力

### 4.2 模板与预算门槛

本轮读取的[候选 commit CMake](https://raw.githubusercontent.com/alibaba/MNN/d407447ed56c4121a11ccbd266dc184ca1ead0c2/transformers/llm/engine/CMakeLists.txt)已始终启用 `LLM_USE_JINJA`，不能笼统写成“3.6.1 默认缺少 Jinja”。但 [3.6.1 tokenizer 实现](https://raw.githubusercontent.com/alibaba/MNN/3.6.1/transformers/llm/engine/src/tokenizer/tokenizer.cpp)在空模板及未启用 Jinja 分支存在内容拼接降级；Nexa 必须拦截这类情况。

准入要求：

1. 锁定模板文本/hash、tokenizer、BOS/EOS 和特殊 token 配置；用首条 system、完整多轮、中文/emoji 验证渲染结果及 token 序列。模板缺失、无能力或执行失败返回稳定错误，不能降级成拼接 messages。
2. 在同一 owner thread 完成 prepare；`prompt_tokens` 包含模板和特殊 token，满足 `prompt_tokens + max_tokens <= logical_context_size` 后才发送 Prepared/Started。图容量、KV 分配取整或硬件上限不改变逻辑预算。
3. 预算检查与生成消费同一 prepared tokens，绑定模型包、模板、加载配置代际；配置变化使 prepared 失效。禁止重复套模板、重复 BOS、字符估算、自动截断或静默滑窗。
4. 先验证固定非思考模式；通过模板上下文关闭思考，不靠追加控制词或剥除输出标签。未通过的模型组合不进入支持矩阵。
5. 覆盖预算刚好等于上限、超出 1 token、空内容、模板膨胀、损坏 tokenizer、缺模板和中途取消。Prepared 之前不得输出任何正常生成增量。

### 4.3 每请求采样不能靠修改 JSON 假定生效

[3.6.1 sampler](https://raw.githubusercontent.com/alibaba/MNN/3.6.1/transformers/llm/engine/src/sampler.cpp)在构造时读取采样配置，并用 `random_device` 初始化随机发生器；[Llm 实现](https://raw.githubusercontent.com/alibaba/MNN/3.6.1/transformers/llm/engine/src/llm.cpp)在加载阶段创建 sampler。后续 `set_config` 不等于已证明现有 sampler 按新请求重建。

T07-B 必须选择并留证：在 owner thread 提供受控 sampler 重建/重置补丁，或实现基于受控 logits 的等价采样。每个请求显式应用 `temperature`、`top_p`、`seed`、输出预算，清理 penalty/RNG/历史状态。temperature=0 使用明确 greedy；固定 seed 在同一锁定环境可复现，随机 sentinel 独立处理，不承诺跨后端逐 token 一致。用“请求 A→不同参数 B→再次 A”和直接 A 对照证明无泄漏，不能仅凭返回了不同文本判定参数正确。契约缺口未修复前只可记为原型，不能宣称接入完成。

### 4.4 取消必须到达实际计算边界

[候选 commit 的 Llm 头文件](https://raw.githubusercontent.com/alibaba/MNN/d407447ed56c4121a11ccbd266dc184ca1ead0c2/transformers/llm/engine/include/llm/llm.hpp)未提供可直接采用的显式 thread-safe cancel API；普通 `LlmContext::status` 和停止状态查询不构成并发取消保证。不得从 UI/控制线程写普通 status、并发调用 reset/destroy 或只关闭 ostream 就报告推理已停。

实现选择须用精确源码和真实模型证明：独立原子标志由 owner thread 在可控 prefill chunk、decode step、可取消输出等待以及加载安全检查点读取；若上游接口不足，采用最小可审查补丁并记录补丁 hash。底层 GPU/NPU kernel 可能不能立刻打断，取消延迟包含当前不可中断调用，不能用“请求已受理”代替“计算已结束”。

验收分别注入加载、prepare、长输入 prefill、decode、慢消费者和前后台竞态；每次确认终态唯一、槽位释放、随后新请求可运行。CPU 小模型目标沿用规格中的通常 1 秒内，实测各阶段最大值/分位数并锁定该配置准入阈值；GPU/NPU 阈值单独制定。超出阈值记失败；原生未返回时保持正在停止，禁止释放资源或开第二个推理实例，不强杀 Android native 线程。普通取消/超时也要在安全清理后才能恢复，无法确认清理时禁止新任务直至实例重建。

## 5. 多文件模型包与可变配置隔离

MNN 模型不是 GGUF 的改扩展名形式。[官方 LLM 文档](https://mnn-docs.readthedocs.io/en/latest/transformers/llm.html)列出图、外置权重、tokenizer、模型配置及可选 embedding 等产物；QNN 还会产生单独的配置和目标图目录。Nexa 要校验运行时实际读取的整个引用闭包，而非只 hash 一个入口 JSON。

### 5.1 待实现 schema 的最小内容

以下为字段职责，不是已可调用的 JSON 协议：

| 字段组 | 内容与校验 |
| --- | --- |
| 身份 | schema_version、model_id、format=`mnn_package`、原模型来源/revision/许可、architecture、精确导出器 commit/参数 |
| 文件表 | 每项规范化相对路径、角色、实际长度、SHA-256；完整 graph/weights/tokenizer/embedding/模型配置/template/固定 context 配置，以及该变体需要的 NPU 图文件 |
| 引用关系 | 已验证入口配置、各配置/图引用的目标文件、必需与可选角色、引用闭包；不允许隐式读取表外文件 |
| 量化与变体 | 权重/激活位宽、对称性、block/group、校准数据公开身份或合成样本 hash、图布局；CPU/OpenCL/QNN/Hexagon 变体分开标识 |
| 兼容范围 | 引擎 commit/ABI、SoC/Hexagon 架构、必要 runtime/驱动条件、逻辑上下文上限、验证证据 ID；未知条件不推定兼容 |
| 完整性 | 文件级 hash 加规范化内容清单整体 hash；定义排序/编码和参与字段，不包含整体 hash 自身，避免自引用 |
| 验证状态 | 未验证/探针/特定设备已验证，绑定 exact package hash、后端、设备档、参数和报告；导入方的自述不能授予 validated |

Windows GGUF schema 1、其单文件 hash 和原证据保持原含义；不能把 MNN 包 hash 填进 `validated_llama_commit` 或 `gguf_file_type`。未来 schema 迁移需覆盖旧 managed/external 注册表读写、未知版本拒绝及中断恢复，不破坏已发布桌面输入。

### 5.2 文件与配置规则

1. 首轮以受控包格式导入到 App 私有持久目录。系统选择器 URI 通过 ContentResolver 读取，不能把 `content://` 当 native 路径；导入源仅只读，不把私有模型上传到转换服务。
2. 检查文件项数、单项及累计大小、可用空间和读取时间；若用压缩包，限制展开规模/压缩比。拒绝绝对路径、盘符/URI、`..`、NUL、重复归一化路径、大小写冲突、符号链接/硬链接及非普通文件。不得跟随来源包指向包外的引用。
3. 所有上游可配置路径（graph、weight、tokenizer、embedding、llm_config、template/context、NPU 子图等）只解析到已校验的文件表项；图内外置数据同样纳入闭包。关闭自动下载、远程资源、任意插件路径与表外默认文件搜索；缺项直接失败。
4. 不把用户提供的完整 `config.json` 原样交给上游。解析允许列表后生成受控有效配置；模型结构/模板/资源引用属于不可变包，backend、线程、功耗、precision、context、请求采样等属于独立且有界的运行配置。未知执行字段 fail-closed，描述性元数据可另行保留但不得传入引擎。
5. 模型身份 hash 覆盖不可变内容清单及文件 hash；运行配置单独 hash/代际并写入实验身份。改变 tokenizer、模板、量化、固定模型配置或图必须产生新包身份，不允许就地修改后沿用旧 validated；改变允许的后端/线程设置不会偷偷重写原包。
6. OpenCL tuning、QNN context cache、临时映射文件写入受控 App 缓存区域，以包 hash/引擎/后端/驱动/配置为 key；不写模型包，不把缓存带入模型完整性证明。KV/prompt 内容默认不落盘。
7. 完成临时导入、全文件 hash/引用验证后，在同一文件系统原子发布整个目录和索引；缺文件、空间不足、中断、重启均不能留下可执行的半包。每次实际 load/重载重新确认身份，避免缓存校验成为永久许可。
8. NPU runtime/DSP `.so` 属于应用依赖闭包，另做来源/hash/ABI/许可核验；模型包不得携带任意可加载 native 代码。不同 NPU 路线的库与模型变体不能靠文件名混用。

## 6. 按最小可验收切片推进

全部切片当前为**未开始**。先完成 T07-A～C 的 CPU 纵向链，再将已稳定的同一契约推广到后端。表中“实现命令/报告”是后续交付要求，不代表今天已有 Android xtask。

### T07-A：锁定版本并运行 MNN CPU 原型

**前置：** 第 1.1 节迁移决策已同步；继续核验授权工具链，所需新许可批准后才获取对应依赖。

**最小交付：** 精确 MNN/导出器/工具链构建锁；一个文本小模型候选包（优先研究 Qwen3-0.6B，实际文件/来源/许可/转换结果另锁）；独立 native CPU 探针；可重复构建和运行的真实命令。先使用公开或合成输入，不添加应用业务。

**通过门槛：**

- 不依赖 Android llama，得到 arm64-v8a native 产物及依赖/ELF/页对齐检查；工具链、ABI 和页大小要求均记录，不以成功交叉编译代替真机加载
- 在公开参考型号对应的实际设备档运行真实中英文与多轮生成；记录模型文件闭包 hash、模板/参数、加载/TTFT/prefill/decode、峰值内存和退出码
- 原型明确隔离缺失的预算/采样/取消能力，不能直接作为产品 Executor；缺设备则保留“构建通过、真机待验证”
- 同一小模型/输入建立后续 CPU 对照；不会引用发布页的吞吐数作为 Nexa 实测

### T07-B：MnnExecutor、模型包和安全契约

**前置：** A 的固定源、包和 CPU 探针可重现。

**最小交付：** 实际 `MnnExecutor` + 自有 C ABI；第 5 节 schema/导入解析器；共同类型迁移；模板、每请求 sampler、精确预算、原子取消、stop/UTF-8、异常清理、唯一终态和有界背压。

**通过门槛：**

- 用真实包完成 store→core→MnnExecutor→shim→MNN 的 A01/A02/A04/A05/A07–A12/A15 对应移动适用部分；模板/采样/取消三个缺口均有明确修复和回归，不能用上游 demo 响应替代
- 对缺文件、错误 hash、引用逃逸、变体错配、未知 schema/配置、导入中断进行负例；模型身份或配置代际变化后旧 prepared 不可执行
- 取消前后新请求、单活动槽+一个移动等待槽、慢消费者、加载失败、卸载竞争、重复创建释放分别留证；CPU 小模型的 prefill/decode 取消不是只在生成前预置标志
- Android link graph 无 llama/PC 宿主依赖；共用类型变更后运行既有 Rust 聚合和 Windows 真实回归，保留旧 GGUF/桌面 external 行为

### T07-C / T08：最小 APK 与前台生命周期

**前置：** B 的核心/安全门槛通过；Flutter/bridge/Android 包版本锁定。

**最小交付：** 一个最小 APK 和独立嵌入接入示例，只有模型包导入、加载、前台生成、停止、状态/错误和脱敏诊断；由既有 core 统一排队，不在 Dart 再维护调度器。

**通过门槛：**

- A21：URI 导入中断、空间不足、授权失效和 App 重启不注册半包，不修改源文件
- A22：后台先停止提交新阶段、取消活动及等待任务，实际安全结束后卸载；loading/prepare/prefill/decode/输出阻塞各阶段均测试。前台恢复不重放旧请求，不自动继续旧生成
- 旋转/Activity 或 Flutter UI 重建时，runtime 所有权与受控句柄代际明确；旧订阅解绑，重复后台通知幂等，已销毁句柄拒绝访问。系统结束进程后重新启动也不恢复旧句柄/队列
- 飞行模式本地生成、依赖闭包、无未授权网络路径实测；UI 线程不阻塞。不得因上游开了 HTTP resource 功能而允许模型配置下载内容
- A19 的 100 次短请求/20 次加载卸载、A23 的 15 分钟持续运行及 A26 内存释放观察按设备独立记录；缺任一必要设备证据则 CPU Android 仍待验证
- 桥接只使用内部/原生事件，不宣称 APK 完成 HTTP/SSE 的 A03；PC worker A13/A14 继续由 Windows 套件承担

### T07-D：OpenCL GPU

**前置：** CPU APK/生命周期基线稳定。

**最小交付：** 独立可选 OpenCL profile、驱动探测、缓存隔离和真实后端报告；CPU 仍随包保留。编译 `MNN_OPENCL` 与配置 `backend_type=opencl` 只说明候选路径，不证明实际执行 GPU。

**通过门槛：**

- 同一设备、相同模型包变体/模板/采样/逻辑 context 做 CPU 对照；若必须重导出或改量化，另立包身份并重新核验质量，不能做成伪同条件对比
- 保存实际算子分配/profile、CPU fallback 原因/范围和后端版本；无法观测时写 actual backend unknown，不能填 validated
- 冷启动编译/tuning 与温缓存分开计时；缓存损坏、驱动变化、分配失败、取消、前后台、卸载和下一请求均覆盖
- A16/A25/A26 与 CPU 共有关键契约通过，额外完成持续运行和热状态记录。失败只禁用该组合；不自动升级到未验证驱动或放宽生命周期

### T07-E：QNN NPU v79 / v81

**前置：** CPU 基础可用、GPU 对照可取得；合法工具链/依赖已经取得，模型量化和目标图导出可复现。

**最小交付：** 两个独立 QNN 能力档及包变体：SM8750/v79 与 SM8850/v81。以实际首测设备档优先执行 v81，v79 有设备后再验收；顺序不意味着两者互相兼容。先完成 CPU fallback 资产和选择语义，再启用 NPU 自动选择。

官方路线通过量化导出和 `generate_llm_qnn.py` 按 `soc_id`/`dsp_arch` 等生成 QNN 图，运行时需要匹配的 QNN/HTP 依赖；其产物与直接 Hexagon 路线分开。[QNN LLM 文档](https://mnn-docs.readthedocs.io/en/latest/transformers/llm.html#qnn-llm)

**通过门槛：**

- 固定 QAIRT/QNN 版本、导出工具、模型量化/校准、图 hash、SoC ID、dsp_arch、chunk/shape/上下文范围和目标 runtime 依赖 hash；核对权重/激活量化、embedding 分离要求。文档示例参数不是冻结配置
- 根据[Qualcomm 支持表](https://docs.qualcomm.com/nav/home/QNN_general_overview.html?product=924033590759186372)和实测能力选择 profile，不仅按手机商品名判断；缺少匹配图或运行库时明确不可用，不把 v79 缓存改名给 v81 使用
- 短输入、长输入、临界 context、模型质量回归、prefill/decode profile 和 CPU 对照完整；区分 CPU 前后处理、混合算子执行与实际 NPU 部分。低精度/图分块收益用测量说明，不承诺 NPU 一定更快
- A16/A25/A26、取消/背压/生命周期重复验证；缺依赖、图不匹配、HTP 初始化失败、热约束、内存失败有清晰结果及受控 CPU 路径，见第 7 节
- SDK 再分发条件和 APK 库闭包已确认，才能进入可交付包；只在开发机导出图不算手机已支持

### T07-F：直接 Hexagon 实验

**前置：** CPU/QNN 对照可复现；Hexagon 工具及 DSP 构建许可/分发范围已确认。保持独立实验标志，不阻塞已通过的 CPU/OpenCL/QNN 组合。

**最小交付：** 独立直接 Hexagon profile、导出变体和匹配 DSP skeleton；先复核上游 v79 参考，再独立构建/验证 v81。3.6.1 发布参考测试不构成 v81 支持证据。

官方当前 LLM 说明限定直接 Hexagon 的 W4 对称量化及 Transformer C4 图；产物还依赖对应的 MNN HTP/DSP 库。[Hexagon LLM 文档](https://mnn-docs.readthedocs.io/en/latest/transformers/llm.html#android-hexagon)

**通过门槛：**

- 明确这不是 QNN 的同义名字；分别记录 Hexagon SDK/Tools、DSP 目标、Host/DSP 库和图/权重 hash，不能把 QNN context binary 当作直接 Hexagon 图
- 对 v79、v81 分别证明构建、加载、执行和取消；架构未知时不试装不匹配 skeleton，不修改系统 DSP 驱动、不要求 root 或绕过 SELinux
- 保存 DSP 算子/profile、Host↔DSP 拷贝和实际混合执行；cDSP 崩溃、RPC 失败、设备失联均记失败，停止该 profile，不能把故障后的 CPU 输出算成 NPU 成功
- 重跑共同契约、质量、持续负载、后台取消、资源释放。只有量测优势且安全/质量门槛通过的组合，才从实验转为可选支持；无优势也如实保留结论

## 7. 能力探测、profile 与 CPU 兜底

能力查询至少区分 `compiled`、`driver_available`、`probed`、`validated`、`selected`，再分别报告 `requested_backend`、`effective_backend`、模型变体/图身份、profile ID、实际算子分配摘要和 fallback 原因。字段及 wire 版本在 ADR/类型迁移时冻结；当前公共状态还不能提供这些 MNN 字段。

1. 默认 CPU。手动指定 OpenCL/QNN/Hexagon 失败时返回明确错误；`auto` 仅从本设备和当前模型包已验证 profile 中选取，不扫描任意文件或根据 TOPS 猜测。
2. 允许的 CPU fallback 只发生在**生成尚未开始**、原后端已安全清理且有明确兼容 CPU 资产时，最多一次，并向 UI/诊断说明请求后端、实际后端和原因。不能偷偷换原模型、量化、模板或缩短 context。
3. NPU 专用产物未必可在 CPU 执行。若 fallback 需要另一变体，应在包/profile 中显式声明且经过质量/身份验证；语义或量化发生变化而不在已批准选择内时，报错并要求显式重新加载，不能假称“同一模型自动兜底”。
4. 内部算子落到 CPU 也必须报告，混合执行不能统称纯 GPU/NPU。配置字符串、成功建图或驱动存在不是执行证明。
5. 生成开始后的后端故障不自动重试；尤其已输出任意文本的请求，绝不从头重放或拼接 CPU 结果。旧请求终态一次，清理确认后由用户发起新 request_id。
6. 探测/加载超时、内存不足及 native 故障不得形成无限重试。不能确认资源安全结束时保持不可接新任务状态；Android 不使用 PC 的强杀 worker 恢复策略。
7. profile 缓存由 SoC/系统/驱动/runtime/MNN commit/模型包 hash/配置身份共同限定；身份变化后重新 probe/验证，不以同一商品名延续旧性能结论。

## 8. 验证报告与晋级规则

每个切片报告包含：Nexa commit、MNN commit/补丁 hash、构建锁、模型包/变体/hash、模板/tokenizer、实际配置、脱敏设备能力档、requested/effective backend、fallback、命令与退出码、用例逐项 pass/fail/skipped/unavailable、证据路径和未验证范围。工具未实现、设备缺失或值不可读取均明确标注，不写 0 或通过。

指标分别报告冷加载、prepare/prefill、decode、TTFT、取消到安全停止、App/native 峰值内存、GPU/DSP 可观测分配、卸载后残留、包下载/安装大小。冷/热缓存分开；固定合成短输入与长输入，至少一次预热、五次正式测量，报告中位数与范围；持续运行比较第 1 分钟和第 15 分钟热状态/速度。不要将模型量化差异包装成后端收益，不引用上游宣传数字作 Nexa 目标保证。

- 纯 Rust/解析/状态单测可使用 fake，模板/token/取消/释放和后端选择必须有真实模型证据
- 交叉编译、native 探针、完整 APK、真机短验、持续稳定性分别记录，任何一层不能替代另一层
- A01–A26 逐项标明 Android 适用部分及 PC 专属部分；缺少必要门槛的后端仍为待验证，不抹去 skipped
- APK 依赖/签名/hash/许可证检查独立于推理结果；不将模型或 SDK 大文件提交源码，不将私有输入、日志或设备标识提交仓库
- 修改公共 core/types/store 后复验 Windows；Windows 正常不证明 Android 通过，Android 成功也不能豁免桌面回归
- T09 以更新后的平台矩阵收口；本轮把 GPU/NPU 纳入执行计划，不自动宣布整个 v0.1 或 Telegram 摘要完成

## 9. 下一条可执行动作与本轮检查

下一条源码任务只启动 **T07-A 的精确版本/依赖清单与 CPU 原型**：方向ADR/规范已同步；接下来核验现有工具链及未获批准的SDK/协议，锁定公开小模型来源并实现可重复native探针。没有真机时可继续依赖审计、构建和独立解析/契约设计，真机门槛保持未完成；不为等待设备而先做聊天业务或另一套 Android llama 引擎。

本轮实施范围为本计划、ADR0008及相关规范/入口文档。检查相对链接、围栏、空白、源码入口与官方引用；未运行Android/MNN构建、模型转换、APK、性能或真机测试，未下载SDK或接受协议。Windows目录版源码及CI由独立任务跟踪，本迁移不更改其实现或验证结论。

## 10. 官方资料及引用范围

检索日期：2026-10-02。上游文档为动态说明；实施时再对所选完整 commit 固化相关源码/命令，不能把 `latest` 视为依赖锁。

- [MNN 3.6.1 发布](https://github.com/alibaba/MNN/releases/tag/3.6.1)：直接 Hexagon 等上游能力和参考测试范围，非 Nexa 实测
- [MNN LLM 文档](https://mnn-docs.readthedocs.io/en/latest/transformers/llm.html)：导出产物、CPU/OpenCL、QNN 与 Hexagon 路线入口
- [MNN 3.6.1 Llm 实现](https://raw.githubusercontent.com/alibaba/MNN/3.6.1/transformers/llm/engine/src/llm.cpp)、[Sampler](https://raw.githubusercontent.com/alibaba/MNN/3.6.1/transformers/llm/engine/src/sampler.cpp)、[Tokenizer](https://raw.githubusercontent.com/alibaba/MNN/3.6.1/transformers/llm/engine/src/tokenizer/tokenizer.cpp)：模板、采样、生成的集成风险证据
- [候选 commit Llm 头文件](https://raw.githubusercontent.com/alibaba/MNN/d407447ed56c4121a11ccbd266dc184ca1ead0c2/transformers/llm/engine/include/llm/llm.hpp)、[CMake](https://raw.githubusercontent.com/alibaba/MNN/d407447ed56c4121a11ccbd266dc184ca1ead0c2/transformers/llm/engine/CMakeLists.txt)：API/并发边界和 Jinja 构建行为
- [Qualcomm QAIRT 支持表](https://docs.qualcomm.com/nav/home/QNN_general_overview.html?product=924033590759186372)：SM8750/v79、SM8850/v81 映射及工具链范围
- [OnePlus 15 官方规格](https://www.oneplus.com/us/15/specs)：公开参考型号；实际系统、驱动及内存另作现场诊断
