# T06 外部模型目录与自动名称契约

2026-10-03更新。ADR0015开放模型基线50c9d41已过WindowsCI并发送，用户目标机仍待验；本轮按[ADR0016](decisions/0016-mixed-model-directory-diagnostics.md)实施混合目录partial/有界诊断和短context扫描默认值，已在43ad5c2通过WindowsCI/包复核并发送，用户目标机待验。下述增量是本轮实施契约，不追溯授予50c9d41新行为。runtime-lifetime lease及轻量启动/首次加载核验保持；实际结果见[本轮验证记录](verification/2026-10-03-mixed-model-directory.md)。

## 1. 现状与最小范围

- 当前真实名称为 `%LOCALAPPDATA%/Nexa/models`（复数），managed 布局为 `models/<id>/{model.gguf,manifest.json}`，另有 `imports/` 与 `runtime/model-store.lock`。CLI serve 以及离线 import/list 都传入同一个 data root；现有配置没有独立模型目录字段。
- 单纯修改 data root 会同时移动 token/config，仍会复制模型，不满足本次需求。
- 保留现有 managed 模型、原始 ID、manifest 与复制导入 API。新增一个外部目录并与旧 managed 列表合并展示，不自动迁移、删除或重命名任何原文件。
- 普通扫描/加载只读，不在外部目录写锁、manifest或缓存。本轮[ADR0017](decisions/0017-model-discovery-and-catalog-download.md)进行中：仅无已配置目录且服务停止时，自动检查真实桌面EXE旁已存在的`models/`并复用扫描；不存在不创建，已配置路径即使missing/stale仍优先，不用CWD、`model/`或其他目录回退。显式选定后方可由独立下载任务创建其自有UUID.part并no-clobber发布新GGUF，原文件不改。
- 目录须为支持的本地普通目录，非递归读取直接子级 GGUF。普通扫描目录本身无需可写，显式下载另需目标可写；配置与元数据写权限仍要求AppData可写。拒绝 UNC/网络/设备路径、路径穿越、任意祖先及最终文件的 symlink/reparse 间接路径。拒绝非普通文件；不扫描子目录、不跟随链接。
- ModelId、精确历史验证证据、模板后推理预算与实例proof/锁/停止约束保持；ADR0015将loadable与validated分离，不以验证矩阵作为型号/hash许可名单。

## 2. 单文件持久化与事务

`%LOCALAPPDATA%/Nexa/model-library.json` 是一个独立、私有、版本化配置与注册索引，保存：schema_version、选定目录的原生路径及 directory_id、library_generation、外部注册记录。外部记录只保存直接子级文件名、生成的 ID、显示名及经核验的模型元数据/身份，不保存任意可执行命令。GGUF 正文不写入 AppData。

- 使用现有私有文件与原子替换机制，不将新设置写入隐藏注释，不承诺两个文件联合原子。既有 config.toml、secrets、runtime 与 managed models 保持位置。
- 应用新目录或重新扫描都必须先确认 runtime 已停止、无存活 discovery，并持有同一 data root 的实例锁直到提交或取消完成。仅卸载不够。不得自动 shutdown 其他客户端。
- 完整枚举、核验和有界诊断先在内存完成，合法集合一次原子发布上述文件；不边扫描边保存。全合法为completed，好坏混合为partial并返回全部内容拒绝诊断。有候选但全部内容拒绝时failed/model_scan_no_usable_files、result=null，保留原目录/index/generation；完整枚举后没有GGUF候选则可completed提交空集合。取消、所有预算触限、IO/路径/reparse/身份变化和保存失败均不部分发布；确定内容错误才可逐文件拒绝，不能静默跳过后称完整成功。
- 无扩展名匹配的普通文件不作为候选；符合本版结构/manifest边界但未在验证矩阵中的GGUF可登记为未实测loadable候选，不再仅因validated=false设为不可用。分片、未知tensor布局、缺模板/损坏文件、模板/key/tensor-name的NUL身份歧义仍拒绝；普通tokenizer值NUL不一概禁止。
- 取消与明确的原子提交决策竞争：取消先赢则不保存；提交决策已赢则完成发布，晚到取消不能谎称回滚。原子保存成功后，独立网络刷新失败不把它改称保存失败。rename已发布而目录fsync失败沿用settings_durability_unconfirmed，明确可能已保存并刷新真实generation，不承诺回滚旧索引。
- 新目录选择不等于长期凭据初始化；保存模型库元数据不得隐式创建或轮换 token。

## 3. 自动名称与稳定 ID

- 新外部记录的 `display_name` 默认取文件名去掉最后一个大小写不敏感的 `.gguf`，保留中文、空格与其它合法文字；空 stem 时用原文件名。复用现有 display_name 最大 1024 UTF-8 字节限制，不截断后假装完整。
- 内部 ID 为 Rust 自动生成的 `ext-<UUID simple>`，仍符合现有 ModelId 校验；与旧 managed ID 冲突则重新生成，绝不覆盖旧记录。WebView 不提供 ID 输入。
- 同一 directory_id、同一直接子级文件名、同一 SHA-256 再次扫描保留 ID。成功刷新发现重命名或内容 SHA 变化时生成新 ID；不凭相同 SHA 猜测移动/重命名关系。
- 相同内容的不同文件可以并存，不自动删除或合并。显示重名使用来源标签/短 ID 区分，不能改磁盘文件名；旧 managed ID 与显示名原样兼容。
- 目录切换成功后，旧 external 注册从当前列表退出；旧 managed 仍在。切回以前目录可产生新的 external ID，本版不维护无限目录历史。

## 4. 已确认的有限预算

以下预算已获父级确认，不能把触限当作只读取前若干项的成功：

- 目录路径至多32KiB UTF-8、64个components（包含平台root/prefix），候选picker与后端复用同一校验；超界拒绝，不截断路径
- 每次目录枚举至多 1024 个直接条目、64 个 GGUF 候选；最多同时核验 1 个文件，坏文件同样计入候选及单文件/总字节预算。成功及软拒文件的guard保留到提交或放弃决定；硬失败确定不可发布并退出后可释放
- 单GGUF至多16GiB；本次候选总字节至多32GiB；metadata沿用64MiB header / 1MiB string / metadata与tensor各100000项 / 数组1000000项等全部既有parser预算；触限一律硬失败。私有read_for_scan用typed来源映射ModelLibraryLimit，不改既有managed read错误语义。W02将同一16GiB单文件限额同步到managed导入前、manifest与load；这不是16GB机器能运行该模型的保证
- model-library.json 至多 4 MiB；所有文件名、显示名、列表及错误记录同时受数量和字节预算约束。本轮新增完整序列化file_errors≤512KiB、完整operation DTO≤1MiB（包含JSON转义），不得截断后伪报partial成功
- apply/scan 总 deadline 300 秒，64 KiB 流式块检查取消及时间；不将大文件整体读入内存
- 运行时核验沿用同一有限循环，不能在 scheduler actor 内做全文件 hash；既有 start/load/stop deadline 不因本功能偷偷放宽
- 每个 bridge 最多一个库操作，最多一个待处理 poll；只保留当前操作和一份有界终态；前端最多 1 Hz、不重叠拉取
- 普通API请求继续采用当前 `runtime_cli::client::REQUEST_TIMEOUT=750s`（包含发送与响应头等待）；外部准备上限300s加既有native load上限300s在此界限内，不能另外叠加无界等待。SSE首响应前的prepare同样受请求/断流取消约束
- 哈希读取的真实进度若展示须明确称为核验，不能称复制进度。首版只要求阶段与已核验文件数，不伪造百分比

## 5. 外部文件的一致性与 lease

外部文件不能沿用“private store 不会被其它程序修改”的旧假设，禁止只凭上次 hash 缓存就把裸路径交给 worker。

本版策略：Windows 在每个 external 模型首次实际加载前，取得只读访问、仅共享读取的文件句柄，拒绝并发写入/删除，并为路径目录取得防替换身份 guard。在同一受保护文件上验证结构、SHA、长度及 OS 文件身份；校验前后身份不一致、已有写者冲突或路径重解析一律失败。worker 仍获得受控绝对路径，但此路径对应的文件/目录保护必须覆盖其随后打开、加载、生成及回收。

启动只读取 ≤4MiB 索引和有界元数据，不重新 hash 外部整库，也不取得全库文件 guard，保持现有 bridge 的 30 秒启动观察界限。首次 `load` 前通过 API 的 storage permit 与 `RegistryLease` 在 blocking 任务中核验单一文件，核验拥有独立 300 秒预算及 64KiB 取消点；成功后才把 guard 发布到 catalog，再调用既有 `runtime.load`。直接 chat 触发自动加载也必须经过同一准备路径，不留绕过入口。原生 load deadline 保持原值，客户端现有 750 秒 request 界限不扩大。当前模型已准备时该步骤只检查有界身份，不重新 hash，也不把 hash 塞进 actor。准备期间断流/取消/服务关停取消该任务，并等实际 blocking 清理后释放 registry lease；未准备完成不能向 actor 宣称可加载。

为避免扩大 scheduler/IPC 所有权改动，本版只对实际准备过的 external 文件持有 lease，由 catalog 保留到整个 runtime 真正停止且 worker 回收；卸载模型不释放外部源保护。UI 明示“替换或重命名已登记源文件前先停止运行”。不得在未确认 worker 回收时提前释放 lease 并宣称已停止。guard 不是可序列化的 ResolvedModel 字段，不通过 WebView 或 IPC 传句柄。

- 注册时的 hash 不代替运行时从受保护句柄的重新核验
- 文件失踪/改动/被写者占用时登记为不可用并给安全错误，不悄悄指向同名新文件；其他 managed 模型仍可查看/使用
- 不在失败时自动复制源文件作为后备
- Linux仅作开发验证，不能宣称其普通 POSIX 文件锁等价于 Windows 强制共享限制；需要明确测试能力或拒绝不具备该保证的 external 推理路径
- Windows 对抗验收必须覆盖注册后替换、已有写句柄、核验/加载时修改、加载后普通写入/重命名、目录替换与 reparse；另观察预存可写mapping等边界并报告真实结果。本版承诺身份/hash及普通写入/替换防护并保留worker隔离，不承诺对任意预存可写映射或同账户恶意行为绝对不可修改，不能靠fingerprint或静默复制夸大保证

## 6. 原生壳与 bridge 固定接口

所有字段 snake_case；有参数的 invoke 仍统一 `{ request: ... }`。以下为冻结的共享签名，前端绝不回传自由路径。

- `model_directory_pick()` → null 或 `{ selection_id, display_path }`。原生 folder picker，Rust保存原路径与选择身份。取消不改变配置；selection只能被一个 apply 接纳一次。壳不自行扫描或 hash。
- `model_directory_apply({ selection_id })` → `{ operation_id }`。壳取出候选路径交 bridge；bridge先登记操作ID，后台完成停止条件/锁/有界扫描/单文件提交。仅真正接纳后消费selection；重复动作返回 desktop_busy。
- `models_scan()` → `{ operation_id }`。按已选目录重新核验；没有选择返回 model_directory_required。与 apply 同样要求停止并持实例锁；在同一次DirectoryGuard中核旧目录身份，路径相同但对象已替换仍硬失败。apply可由新的显式selection选择不同目录。
- `model_library_next({ operation_id })` → `{ operation_id, status, phase, examined_entries, candidate_files, verified_files, failed_file_name, file_errors, terminal, result, error }`。status=`running|completed|partial|cancelled|failed`；phase=`checking|enumerating|verifying|committing|finished`；result=null 或 `{ library_generation, directory_id, registered_files, available_files, rejected_files }`。completed/partial必须有result且error=null；partial为已提交合法集合，rejected_files与完整file_errors一致。全坏为failed/model_scan_no_usable_files且result=null，不发布；硬失败只保留已观察诊断，不声称后续文件已检查。
- `file_errors`为`[{ file_name, code, message }]`，仅invalid_manifest/unsupported_model/unsupported_chat_template及静态受控message。basename≤1024 UTF-8字节，不含目录；`failed_file_name`仍可指具体硬失败basename。原生授权UI可显示，日志/CI不写文件名。旧completed缺file_errors/rejected_files按[]/0读取。诊断不持久化，仅当前App生命周期内一项操作和一份有界旧终态；重启不承诺找回。
- 每次next最多等1秒，不传模型正文、原始异常或完整路径。terminal在工作/实例锁释放后发布；重复读取返回同一有界结果。soft-reject guard覆盖最终提交或放弃决定，不为等待UI poll继续持有已确定硬失败的资源。
- `model_library_cancel({ operation_id })` → `{ operation_id, status:"stopping" }`。只取消本UI持有操作；真正终态由 next 返回。早取消不得丢失，不得取消另一个UI操作。关闭UI复用此取消并有限等待真实扫描结束，不能提前释放仍被blocking任务持有的实例锁。
- 原 `model_import` 与公开复制导入 HTTP/CLI 保留兼容；新桌面主流程不要求手填 model_id，也不把扫描叫做复制导入。

Rust facade：`directory_apply(self:&Arc<Self>, path:PathBuf)->Result<LibraryOperationHandle>`、`models_scan(self:&Arc<Self>)->Result<LibraryOperationHandle>`、`async library_next(Uuid)->Result<LibraryOperationState>`、`async library_cancel(Uuid)->Result<LibraryStopping>`。path仅来自shell原生selection，不建立自由路径invoke。

## 7. snapshot、分页与现有API兼容

snapshot新增：

- `model_directory: { configured, effective, state }`
- configured=null 或 `{ directory_id, display_path, library_generation }`，来源是私有model-library.json
- effective=null 或 `{ directory_id, display_path, library_generation }`，只能来自同TCP proof后实际运行服务的状态。停止时effective=null；不能用磁盘新配置冒充运行实例已采用
- state=`default|ready|stopped|stale|missing|unavailable|unsupported`；当前操作过程由library_next给出，错误不抹掉configured
- `runtime.selected_model_display_name: string|null`，由实际服务从注册表取值，独立于当前模型分页

`ModelsPage`增加`generation`（UUID）；Rust facade改为 `async models_page(after:Option<String>, generation:Option<Uuid>)->Result<ModelsPage>`。`models_page({after,generation})`第一页两者null；后续页带上次generation。列表generation变化返回`model_list_changed`，UI丢弃旧页从第一页重新取，不无限缓存分页。本版停止时可保留带旧generation的视觉占位，但不得当作可加载列表；扫描完成先显示摘要，启动后重新取实际列表。

`ModelSummary`继续复用原id/display_name等字段，增加storage=`managed|external`及availability_error可空受控code。显示名为主，ID只在详情。运行时模型库身份与所选模型显示名可作为`/runtime/status`的向后兼容新增字段；`/runtime/models`增加generation，游标查询generation为可选，旧HTTP客户端原请求仍可用。旧服务没有新增字段时：没有external配置可明确显示legacy managed状态；存在external配置必须显示unsupported/需停止后启动匹配版本，不能伪称目录已生效。新桌面的models_page必须获得真实UUID generation；旧服务缺失时返回model_library_unsupported要求重启匹配版本，不伪造合成generation。旧公开API调用者对新版服务的原请求仍兼容。

新服务从启动时实际读取的catalog保存effective身份；其他客户端改配置或另一实例启动导致library_generation不同，bridge清空已选模型/分页缓存并显示stale，不偷换模型继续生成。模型加载/聊天仍按现有ID及已证明实例操作，不按前端展示文字选模型。

### 模型兼容性说明（Windows）

按[ADR0015](decisions/0015-open-model-loading-and-validation-evidence.md)，当前开发增量分开三个维度：

| 字段 | 语义 | 不能据此推出 |
| --- | --- | --- |
| validated / validation / capabilities | 原有精确模型/参数/设备的历史验证证据 | 其他模型禁止加载、任意context/设备已验证 |
| loadable | 有效受控manifest的独立候选资格；实际提交native前另须文件准备，不是当前完整性证明或型号/hash白名单 | 实际架构/原始模板/设备资源必成功 |
| available / availability_error | 候选资格与当前观察到的源/准备错误，具体错误优先 | 永久文件完整性或模型质量 |

ModelSummary新增loadable和候选context_limit信息；manifest schema1保留验证字段与伪造声明拒绝。列表计算只用有界元数据，不增加整库hash；每次实际load/chat仍走源保护和完整性准备，不能由UI自行授予资格。

compatibility保留字符串兼容读取；新写入以admitted表示有原精确历史证据、unvalidated表示未实测，旧architecture_unsupported等枚举可读取，不再依赖它们作为人工模型名单。旧服务缺loadable时UI保守禁用并要求启动匹配版本，不把旧validated当新loadable伪造。

未实测合法候选显示“未实测，可尝试加载”；实际引擎、模板、输出framing与资源限制仍可导致失败。文件/目录消失或变动等具体错误优先；不把失败改称历史未入名单，也不静默换模板。旧389eeef的UI/字段门槛保持历史事实；ADR0015增量已由50c9d41 WindowsCI回归并发送，新包原生窗口/用户目标机仍待验。本轮ADR0016扫描增量另行验证。

context_limit按模型metadata和131072既有硬限约束；历史验证context2048只表明测过2048，不能阻止合法更大参数尝试，也不保证16GB能容纳131072。没有新增Job RAM硬限制，UI须如实提示内存/资源风险。

### 混合目录增量与已交付版本边界

已交付50c9d41仍可能因单个坏GGUF整批失败。本轮ADR0016在完整安全扫描后，将明确内容拒绝与硬失败分开；合法集合一次原子替换并标partial，全坏保旧index/generation，空目录可空提交。被拒文件仅列本轮诊断，不持久注册为不可用模型。成功partial不混入旧坏条目，未扫描部分不冒充完整。

仅自动扫描的登记默认context改取min(2048, metadata.context_length)；显式import/load和UI参数不夹紧。扫描登记成功不保证当前加载参数适合该模型。该增量已有43ad5c2 WindowsCI及包发送证据，目标机待验，不追溯修改50c9d41事实；新增自动发现/下载另按ADR0017验证。

## 8. 默认发现与目录内下载增量（ADR0017，进行中）

`model_directory_discover()`返回null或既有LibraryOperationHandle；只接收原生壳绑定的EXE/models，不接收前端自由路径。已有配置绝不回退；未配置、停止且目录存在时复用完整有界扫描，随后状态/partial/失败与generation仍按本契约。

固定8条catalog只提供下载建议，不构成模型许可名单；启动/展示不联网。显式`model_download_start({catalog_id})`在接纳时冻结已保存download_source与目录身份，必须服务停止；不自动stop、切源、重试或登记。下载进度与取消使用独立operation，不复用scan结果伪造状态。

仅写自有`.nexa-download-<UUID>.part`，全量实际size/SHA256匹配后原子no-clobber发布；已有同名文件或竞争新目标不覆盖。成功saved=true/registered=false，用户须再扫描；失败/取消清理只针对本任务实际持有对象。发布后清理未确认仍承认saved并警告，不谎称回滚。普通扫描忽略非GGUF.part但计目录条目，源保护和模型结构检查不变。

按[ADR0041](decisions/0041-declared-payload-startup-validation.md)，包启动校验不再扫描或拒绝任何未声明文件、目录和下载残留；不执行或自动删除。声明产品文件及祖先的reparse防护、必需文件和manifest/hash保持，下载操作自身的边界不变。完整网络/设置回退/任务上限见[ADR0017](decisions/0017-model-discovery-and-catalog-download.md)，结果见[本轮记录](verification/2026-10-03-model-catalog-download.md)。

## 9. 受控错误与验收

复用 runtime_running、desktop_busy、request_not_owned 等既有错误；新边界需要明确安全code：model_library_unsupported、model_directory_required、model_directory_unavailable、model_directory_unsupported、model_library_limit、model_library_changed、model_list_changed、model_scan_timeout、model_scan_cancelled、model_scan_no_usable_files、model_file_changed、model_file_unavailable、model_file_in_use、model_library_write_failed。错误正文不包含token、完整源路径或任意底层异常。

至少覆盖：非白名单模型的loadable/validated分离、伪造证据拒绝、旧服务缺字段保守处理；混合目录partial/全坏保旧/真空清空、诊断完整与预算、parser预算硬失败、scan-only旧身份、软拒guard与短context仅扫描默认值；中文/空格文件自动显示名；旧managed可见/ID不变；只读目录零写入且GGUF hash/长度不变；新目录不复制；同名/重命名/同hash多文件规则；取消/失败原配置与索引不变；目录/文件消失；超限不部分提交；旧runtime身份不匹配；分页generation拒绝陈旧页；Windows外部文件lease对抗矩阵；真实外部GGUF加载/生成/取消/卸载/停止；停止后原文件可再次编辑；程序旁目录不会令严格产品inventory把合法模型误拒，也不会放行任意DLL/EXE/reparse。

壳包校验只给指定数据类别建立准确边界。不能整棵`model/`或`models/`无条件跳过；所选目录在产品外无需改产品清单；产品内部的源GGUF目录只允许受控目录/普通GGUF与必要的精确规则，产品EXE、runtime、DLL、manifest及SHA清单仍原样严格验证。新目录注册元数据不在产品里，因此无须为model-library.json/imports/model-store.lock放开发行包。

## 10. 分工

- 父级：确认限额/lease与最终契约、整合审阅提交；既有14文件修复保留各自写入者，可与本功能一起完成验证后统一提交
- bridge工位：model-store外部catalog及共享元数据验证、必要runtime-types/API/CLI/bridge修改与对应harness；独占根Cargo/锁；尽量不动native/worker IPC
- shell工位：native目录选择/一次性selection、固定invoke/ACL、包内GGUF目录精确规则、Windows生命周期/共享访问对抗与打包证据；不自行扫描
- UI工位：设置选择/应用/扫描/取消状态；显式先停止流程；显示名为主、来源/重复名区分；generation分页及selected_model_display_name；移除新流程手填ID

实施已获父级授权；本次不移动真实用户文件、不初始化长期真实凭据。
