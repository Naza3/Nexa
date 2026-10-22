# ADR0025 统一运行档案与配置版本

日期：2026-10-04。状态：用户已批准整体方案，代码实施与验证中。基线为6aa0e1f；本决策不表示下列能力已经交付。

## 目标与保持不变的边界

使用一个共享解析器和权威配置源，让UI和API的明确加载遵循本次覆盖、模型档案、全局默认的优先级。保留原actor会话快照、单驻留模型、worker隔离、具体ModelId IPC和LAN只允许调用已加载模型。

复用config.toml并升级schema2；不建立额外数据库。运行中的profile和请求默认可CAS保存，后者只影响新解析请求。全局加载、监听、idle、文件校验和推理执行超时策略仍停服保存。迁移冲突必须由用户选择，不重置数据或凭据。

下面为已冻结v1.1实施合同；若实现发生必要调整，须先同步此决策和跨层测试。

## 1. 磁盘schema / 迁移发布点

保留现有[api]、[lan_api]、[runtime]、[inference]的字段名称与验证范围。schema2的增量例子：

```toml
schema_version = 2
# api / lan_api / runtime / inference 同现有格式
[inference]
backend = "cpu"
context_size = 4096
max_output_tokens = 512
temperature = 0.7
top_p = 0.9
gpu_layers = 0
# 未写threads仍代表automatic
batch_size = 512

[model_profiles.qa-small]
context_size = 2048
threads = 2
batch_size = 128

[migration]
legacy_config_revision = "sha256:<64 lowercase hex>"
legacy_preferences_revision = "sha256:<64 lowercase hex>"
choice = "desktop"
```

- `model_profiles: BTreeMap<ModelId, LoadOverrides>`；每个字段Option<u32>，缺省=继承，不持久化null。全空档案删除该map条目。最多128档案，整个config仍≤65536字节，超过失败，不截断。
- `migration: Option<MigrationReceipt>`仅迁移时出现；新建schema2无回执。receipt里的两revision+choice够证明来源，无需存路径/秘密/原始全文件；choice为api/desktop/custom/equal。
- `Config::default()`仍为schema1以兼容未显式声明版本的旧TOML；新初始化按上述条件明确选择schema2。`Config::from_toml`接受schema1与2；schema1禁止承载非空profiles/迁移回执。未知版本、未知字段、重复字段/ID、非法旧TTL全部fail closed。读不写、不迁移、不建锁文件/目录。
- schema1启动/CLI/旧API在用户尚未选择时保持旧API默认语义，profile编辑返回configuration_migration_required。
- `desktop-settings.json`迁移后只承载UI偏好。迁移本身不改这个文件：schema2是唯一权威开关，旧推理值此后忽略。这样只需一次config原子发布，不伪称两文件事务。
- 下一次明确UI偏好保存可以写schema2的`{schema_version:2,close_runtime_on_exit:false,download_source:"modelscope"}`，旧文件读兼容仍保留。新UI保存不得携带推理字段；旧settings DTO另见兼容节。

## 2. 同一服务中的有效快照，避免缓存第二真相

`ApiState`保留启动Config（端口、安全、runtime budgets），新增内存active配置快照（schema/revision/global/profiles）。服务构造时从同一次已锁定读取初始化；所有在线桌面、HTTP显式load、cold chat都从这个active快照走同一resolver。

- 服务运行时桌面configuration_get/save必须经已证明身份的loopback管理连接；禁止UI读磁盘global当运行有效值。
- 在线profile/request_defaults保存：配置事务序列化；获取配置文件锁、重读及比较CAS、验证磁盘revision仍与active快照一致（不能捎带导入外部热字段）、原子发布，然后更新active对应快照；返回前保证可见。新load克隆完整旧或新快照，不得混合两版字段。
- 配置get同时观察磁盘与active；如果外部编辑了文件，返回saved、runtime_effective与pending_restart，不自动把外部文件变为热配置。外部直接编辑profiles同样需重启或经管理接口重新明确保存；不能偷偷绕过协议刷新一半字段。
- 磁盘revision与active不一致时（包括外部只改profile/request defaults），在线save返回configuration_restart_required，零配置写入。既有生成、排队任务和同selected idle恢复继续原会话；旧HTTP显式load至多使用active快照，不使用尚未生效磁盘global。UI将reload标为需重启，不显示“已应用”。
- active配置快照只有schema2管理profile/request_defaults提交可热更新。request_defaults只替换max_output_tokens/temperature/top_p；HTTP请求体读取后、parse_chat前克隆同一受控快照，解析出的GenerationOptions即冻结，后续排队或profile变化不得再改。保存成功应答后开始解析的请求必须看到新默认；与保存并发的请求可完整看到旧或新快照。

## 3. JSON DTO（准确英文命名；实现可加Rust内部辅助类型）

### 3.1 共用类型

```ts
type Revision = string; // "sha256:" + 64 lowercase hex；absent只用于未初始化预览
interface LoadDefaults { context_size:number; threads:number|null; batch_size:number }
interface LoadOverrides { context_size:number|null; threads:number|null; batch_size:number|null }
interface LoadOptions { context_size:number; threads:number; batch_size:number }
interface RequestDefaults { max_output_tokens:number; temperature:number; top_p:number }
interface RuntimePolicies { execution_timeout_seconds:number; idle_unload_enabled:boolean; idle_unload_seconds:number; model_verification_timeout_seconds:number }
interface ProfileEntry { model_id:string; load_overrides:LoadOverrides }
interface ConfigurationValues {
  global_defaults:LoadDefaults;
  request_defaults:RequestDefaults;
  runtime:RuntimePolicies;
  local_api:{listen:string};
  lan_api:{enabled:boolean;listen:string|null;allowed_cidrs:string[]};
  model_profiles:ProfileEntry[];
}
interface MigrationDifference {
  field:"context_size"|"threads"|"batch_size"|"max_output_tokens";
  api:number|null; desktop:number;
}
interface MigrationStatus {
  state:"not_needed"|"legacy_compatible"|"required"|"complete";
  preferences_revision:string|null;
  differences:MigrationDifference[];
  backup_available:boolean;
}
interface ConfigurationSnapshot {
  schema_version:1|2;
  revision:Revision; // 当前磁盘config版本，供CAS；未初始化为"absent"
  saved:ConfigurationValues;
  runtime_effective:{revision:Revision;chat_response_timeout_seconds:number;values:ConfigurationValues}|null;
  pending_restart:boolean;
  migration:MigrationStatus;
}
interface ModelConfiguration {
  configuration_revision:Revision;
  model_id:string;
  load_overrides:LoadOverrides;
  saved_effective:LoadOptions;
  saved_sources:{context_size:"global"|"profile";threads:"global"|"profile"|"automatic";batch_size:"global"|"profile"};
  current_load_options:LoadOptions|null;
  restore_load_options:LoadOptions|null;
  pending_apply:boolean;
  context_limit:number|null;
}
```

- profile null=继承；global threads null=automatic。profile不能单独指定automatic，恢复继承即随global。
- `current_load_options`仅actor Ready/Generating且selected匹配时非null。`restore_load_options`仅该selected匹配而非驻留时显示历史options；不能用它显示“当前加载”。
- `pending_apply`在驻留时比较真实options与saved_effective；无驻留时false，UI写“下次主动加载使用”。same selected idle的恢复说明使用restore值。
- sources只描述saved解析值；真实options来自session snapshot。不能从值相等猜测真实加载来源或伪造精确已应用revision。
- `context_limit`来自已有有界库存metadata，不读/哈希GGUF。已知上限保存/加载皆校验；未知明确标未知，不发明数值。
- DTO不包含token、token_file、源文件完整路径、可信origin任意编辑口、system/raw error。
- 按[ADR0034](0034-configurable-execution-timeout.md)，`runtime.execution_timeout_seconds` 是唯一新增可编辑超时，默认300秒。统一runtime整组JSON更新必须显式携带该字段，范围1..86400；缺省、null或类型错误拒绝，不用默认值覆盖用户已有设置。旧TOML缺省仍取300，已有合法超大正数继续可读，不阻塞其他组保存。保存runtime组超界返回 `configuration_invalid`，param 为 `update.runtime.execution_timeout_seconds`。
- `runtime_effective.chat_response_timeout_seconds` 是只读预算：从实际active配置的文件校验、排队、加载、执行秒数依次饱和相加，再饱和加30。磁盘pending值不参与，溢出饱和至u64上限仍允许GET；桌面转成单调时钟截止点不可表示时返回 `response_invalid`。该字段不是第二个用户设置，stream预算由同一effective执行秒数加30派生。

### 3.2 CAS写请求

```ts
type ConfigurationUpdate =
 | {kind:"model_profile";model_id:string;load_overrides:LoadOverrides}
 | {kind:"global_defaults";global_defaults:LoadDefaults}
 | {kind:"request_defaults";request_defaults:RequestDefaults}
 | {kind:"runtime";runtime:RuntimePolicies}
 | {kind:"local_api";local_api:{listen:string}}
 | {kind:"lan_api";lan_api:{enabled:boolean;listen:string|null;allowed_cidrs:string[]}};
interface ConfigurationSaveRequest { expected_revision:Revision; update:ConfigurationUpdate }
interface ConfigurationMigrateRequest {
  expected_revision:Revision;
  expected_preferences_revision:string|null;
  choice:"api"|"desktop"|"custom";
  custom:{global_defaults:LoadDefaults;request_defaults:RequestDefaults}|null;
}
interface ModelLoadProfileRequest {
  model_id:string;
  load_overrides?:{context_size?:number;threads?:number;batch_size?:number};
}
interface UiPreferencesSaveRequest {
  expected_revision:string;
  preferences:{close_runtime_on_exit:boolean;download_source:"modelscope"|"huggingface"};
}
```

- Save每次只有一个kind，保持影响和锁需求明确，返回完整ConfigurationSnapshot。成功发布但fsync失败返回configuration_durability_unconfirmed，客户端先重读，禁止自动重复提交。
- runtime策略只允许停服持锁CAS保存，执行超时重启服务后生效；运行中提交返回 `runtime_running`，不改变active计时。
- UI只提交用户本次确认的组。每组全量字段受CAS保护；冲突保留dirty草稿，重读比较后仅重新应用已确认字段，不能用新revision自动重发旧全表单。
- `model_load_profile`平时只传model_id；只有“本次临时覆盖”发load_overrides。不能把显示出来的saved_effective全部回传成request覆盖，否则profile来源被永久遮蔽。
- 临时load_overrides的null非法，缺省=未覆盖；backend/gpu固定cpu/0，不添加新选择。

## 4. 新命令与管理路径（保留旧入口）

| Tauri命令 | request | 返回 | 行为 |
|---|---|---|---|
| runtime_initialize | 无 | DesktopSnapshot | 初始化私有根与local token；全新无旧偏好时写schema2，存在旧偏好但缺config时保留schema1等待明确迁移；不启动进程、不绑定端口、不创建LAN token、不扫描；既有legacy preferences走第6节 |
| configuration_get | 无 | ConfigurationSnapshot | 在线走management；离线同共享读取 |
| configuration_model_get | {model_id} | ModelConfiguration | 档案编辑器与当前/保存对照 |
| configuration_save | ConfigurationSaveRequest | ConfigurationSnapshot | profile/request_defaults在线，其余明确停服 |
| configuration_migrate | ConfigurationMigrateRequest | ConfigurationSnapshot | 只离线；原子升级 |
| model_load_profile | ModelLoadProfileRequest | RuntimeStatus | 解析+主动load+既有安全短测，忙则拒绝/短测延期 |
| ui_preferences_get | 无 | {revision,preferences} | 与runtime配置独立CAS |
| ui_preferences_save | UiPreferencesSaveRequest | {revision,preferences} | 不写推理字段 |

新命令沿用Tauri `{request: ...}`包装约定；无request命令不传空对象。`runtime_start({initialize_if_missing})`保留；新UI首次设置用initialize而不是start→stop。

HTTP仅加到`routes::router`，不加到`lan_router`：
- `GET /runtime/configuration` → ConfigurationSnapshot
- `PUT /runtime/configuration` → ConfigurationSaveRequest；在线只接受model_profile或request_defaults，其他kind返回runtime_running
- `GET /runtime/configuration/models/{model_id}` → ModelConfiguration
- 无HTTP initialize/migrate（需停服时本机bridge/CLI直接共享模块）

`POST /runtime/load`、`/runtime/load-and-test`、`/runtime/load-if-unloaded`仍接受原flat字段：`model`, optional `context_size`, `threads`, `batch_size`, `backend`, `gpu_layers`。省略的字段改用统一resolver。旧显式字段优先级不变。不扩展`/v1/chat/completions`允许字段或LAN能力。

`DesktopSnapshot`新增`configuration?:ConfigurationSnapshot`、`configuration_error?:BridgeError`及`ui_preferences?:{revision,preferences}`，其余字段保留。配置能力读取失败时保留已证明的runtime/status，错误单独展示；旧服务缺少新路由不拖垮查看或明确停服，也不能用磁盘/TS伪造新配置。新前端若bridge不支持新命令，显示功能不可用而不能在TS自己仿造档案持久化。旧settings投影在schema2从global/request+UI偏好合成；迁移pending时仍保留旧桌面投影与旧API各自语义并清楚提示。

## 5. CAS、锁、安全与失败边界

- revision定义为当前config原始完整字节SHA256，不写回revision字段。能检测未遵循工具协议的外部编辑，避免单纯计数器漏检。同内容no-op保持revision。
- 配置独立锁文件`runtime/configuration.lock`。由private create-new创建，一旦创建不更换inode、不删除、不锁config本体（原子rename会替换inode）。读已存在配置时可shared lock；只读缺锁不创建文件，依靠原子发布读取完整bytes。
- 官方写者必须统一使用exclusive ConfigLock。离线管理按InstanceLock → ConfigLock；在线API已持InstanceLock生命周期，再拿ConfigLock。profile/request_defaults在线保存不申请第二把InstanceLock。禁止反向锁顺序，禁止持ConfigLock做模型哈希、网络等待、actor等待或加载。
- 每次写：锁→重读有界安全文件→比较expected_revision（及迁移preferences_revision）→校验更新后的完整配置/有界model metadata→序列化限长→私有临时文件write+sync→平台原子replace→父目录sync→更新active snapshot→应答。CAS冲突前不建备份/临时/更新任何业务文件（锁文件本身建立是协调元数据，不应称为配置改动）。
- 适用于本程序所有官方写者的线性CAS保证。任意外部编辑器不遵守锁时无法许诺跨进程强CAS；支持外部编辑的规则为先停服务，保存后重读，不宣传运行时手改热应用。
- 必须复用/下沉`settings::atomic_replace`至runtime-api共享实现；Windows保留MoveFileExW(REPLACE_EXISTING|WRITE_THROUGH)，Unix rename+directory sync，绝不remove再rename。
- 复用token平台私有目录/句柄校验与现有安全文件规则，拒绝symlink/reparse/硬链、不可信私有权限。不要仅用exists+open而放松新配置锁。旧CLI init/start也必须接共享模块，不能漏一个不拿ConfigLock的writer。
- 不可因损坏/无法读取就重新初始化、清模型索引、重置credentials；不自动停运行服务。停服失败或discovery残留返回runtime_stop_unconfirmed。
- UI偏好也要独立文件revision+同ConfigLock串行，避免两窗口覆盖；旧推理字段迁移输入的读取和第一次迁移比较必须在相同锁内。

错误码（HTTP保持现有error envelope，Tauri BridgeError只给safe code/message；冲突后GET得到新版，不在错误中回传文件内容）：
- configuration_revision_required，409；拒绝无版本的旧写客户端
- configuration_conflict，409；零配置写入
- configuration_migration_required，409
- configuration_restart_required，409
- configuration_busy，409；不无限等待
- configuration_invalid / model_profile_invalid，400；param标具体字段
- configuration_unavailable / configuration_write_failed，500
- configuration_durability_unconfirmed，500；可能已发布，先重读
- runtime_running / runtime_stop_unconfirmed，沿既有语义

## 6. 迁移具体协议

1. 只读preview读取config schema1及真实存在的旧desktop-settings。无旧preferences文件不制造已保存的“2048/2/128偏好”：以API为已保存唯一来源，状态legacy_compatible。如果首次initialize前已经有真实旧preferences，必须保留为迁移输入：只初始化schema1默认config并返回迁移预览，不直接创建schema2忽略旧值；全新无旧preferences才创建schema2。旧文件损坏则阻断迁移，不用默认掩盖。
2. 只比较四个重叠字段：context_size/threads/batch_size/max_output_tokens。api threads automatic与desktop显式数值即使本机算出相等仍属于差异，避免误抹掉自动策略。temperature/top_p沿原API（旧桌面无此字段）。
3. 有差异返回required+differences，零写，旧API继续原行为。新profile/global保存被阻止；用户显式选择API、desktop或custom。已完成schema2绝不再导入旧desktop文件。
4. stop完成并拿InstanceLock后，以两个原始来源revision做CAS；nullable preferences_revision表示原文件不存在，若后来出现也冲突。
5. 校验最终global（含batch/context交叉约束）与每个已有profile。在所有校验/CAS通过后，为原config和存在的旧preferences各保存private不可覆盖备份，固定来源revision命名在私有`backups/configuration/`，不含凭据文件；有同名备份时校验bytes一致，否则失败。
6. 只原子发布一个schema2 config：原非推理字段原样保留+所选global+profiles空+receipt。发生崩溃只能是旧config或完整新config；多出的备份不代表迁移完成，receipt才代表。桌面原文件不变。
7. equal/无legacy的首次明确save可在同一事务升级schema2并写所请求变化，不因GET升级。legacy_compatible也应告知将写schema2和回退限制。
8. 降级旧程序可能拒绝schema2，应明确备份恢复需停服；不承诺新schema向旧程序可读，不自动降级。

## 7. 精确源码接点与Actor改动

### runtime-api / CLI

- `config.rs:Config/validate`扩schema；`configuration.rs`新增共享读写、迁移preview、LoadOverrides、ConfigurationSnapshot和唯一`resolve_load_options(config, model_id, explicit)`纯函数。
- `dto.rs:LoadRequest::options`改为使用模型profile；保留旧请求解析/严格字段检查。可保留现有`options(&Config)`签名，Config已拥有profiles。
- `routes.rs:load`、`desktop_load`、`model_test`不再直接读取`state.config`解析加载参数：统一从active配置快照解析。model_test若是对当前模型验证应优先真实options，不能拿刚保存未应用的profile标错scope。
- `ApiState::load`与`probe.rs::desktop_load`接收同一次解析出的LoadOptions；原external prepare、registry lease、TOCTOU、忙检查、短测submit_if_idle全保留。explicit_threads可改为准确source传参，不再按startup config猜profile threads。
- `ApiState::submit(GenerationRequest)`在外部文件准备完后克隆active配置并resolve该model的cold候选，然后调用新的runtime handle方法。profile changes in flight线性化于快照读取；一旦actor接受，后续profile更新不改变该请求/会话。
- `ApiState::submit_current`与`submit_loaded`不能读取status猜selected、不能触发prepare/load；保留既有原子路径。chat sampling显式值/校验/SSE保持。`chat.rs::chat_request`现有481行将`parse_chat(&bytes,id,&state.config)`改为先克隆active config再parse_chat；本机/LAN共用这个入口，因此请求默认同时生效。ApiState启动config仍用于网络/限制等非热项。不得把generation defaults在worker开始执行时重新解析。
- `runtime-cli/src/command.rs:serve`在持InstanceLock后一次读取config，传同一份至RuntimeConfig和ApiState。init使用共享初始化但不自动迁移未知损坏配置。CLI命令flag与旧API payload不改。

### runtime-core

最小新增：`RuntimeHandle::submit_with_load_options(request: GenerationRequest, cold_options: LoadOptions)`与`Command::SubmitWithLoadOptions`。原submit保留，作为调用同一内部submit实现并使用RuntimeConfig.load_options的兼容入口。

`Actor::submit`公共准入顺序保留：stopping/registry/fault/duplicate/model_conflict/queue_full先检查；仅`self.selected.is_none()`时校验及消费cold_options并resolve。已有selected同ID无论Ready、Generating或idle Unloaded都忽略新的cold_options；旧session options不被保存profile污染。不同ID仍model_conflict，不能借profile实现隐式切换。`explicit_load`继续既有忙拒绝、串行卸旧/装新和精确options no-op。

主动reload明确发一个load控制命令并重新解析最新active profile；相同ID和相同值可保持现有no-op（逻辑上临时覆盖结束但资源不必无意义重建）。若必须强制重建模型是另一个动作，不伪装为本契约要求。

### desktop-bridge / Tauri

- `lib.rs:snapshot_inner`schema2 settings投影不再读旧DesktopPreferences的推理值；真实驻留只由status.state+selected派生。
- `start_inner`抽出无进程副作用initialize；新UI initialize配置后才由用户启动。原start(true)兼容，只在确实缺文件时创建，不拿任何错误当not_initialized。
- 原`settings_save/save_runtime_config/save_lan`必须纳入共享ConfigLock，不能残留不带CAS的新UI调用。legacy协议见下一节。
- 新`model_load_profile`复用原load_model生命周期/关闭取消保护，但构造HTTP body时只发用户指定的临时字段；原load_model保留必填旧DTO并视为本次显式覆盖。
- `windows.rs`新命令全部使用现有guard；normal DTO无credentials。本机/LAN复制token继续native only。

## 8. 兼容策略（安全优先）

- 老HTTP /v1 chat、/runtime/load flat字段与CLI flag照常；新profile统一解析遗漏字段是新已批准行为。空model、null拒绝、显式错ID不回退照旧。
- 老Rust core submit保留，新增方法只给API冷加载入口；worker IPC/public GenerationRequest不变。
- 老Tauri `settings_save`无revision不能满足新CAS：schema1仍可用旧流程，但须ConfigLock；schema2对包含推理字段的旧settings_save明确返回configuration_revision_required，不静默写入global或退回第二来源。新UI必须使用新命令。旧idle/LAN命令同理，新UI停止调用；必要兼容read-only与start/stop/load不受影响。
- 这是旧写客户端的明确可见升级保护，不能把后端“临时读最新revision再替老客户端保存”冒充CAS。

## 9. 验收矩阵 / 同时实现交付界线

后端最低测试：
1. schema1不写、未知schema拒绝、schema2 profiles往返、defaults自动线程、继承/覆盖/清空、cross-field与metadata上限。
2. A/B不同profile分别UI/load API/cold chat得到相同有效值；显式覆盖最高且不回写；profile保存中Ready/Generating不中断。
3. request_defaults在线CAS后下一请求省略字段读新值、显式值优先、已解析/排队请求保持旧GenerationOptions，本机和LAN均覆盖；actor同selected idle恢复旧options；主动reload新options；cold重启新options；显式错ID不换；empty/current路径并发仍原子且无profile文件I/O。
4. 两个相同revision并发保存只有一个成功；失败方无业务写；外部编辑hash冲突；跨profile/全局保存不覆盖无关字段；online/offline writer竞态。
5. migration api/desktop/custom/equal/absent/corrupt、source-change冲突、backup失败、rename前后故障、receipt重读不二次导入、ID/token/index bytes不变。
6. 服务运行中global/idle/LAN保存拒绝；profile/request_defaults可保存；disk外改global显示pending_restart且在线解析仍active旧global，重启后共同切到新global。
7. private权限/锁路径symlink/reparse/硬链拒绝；LAN管理路径404；LAN仍独立key/CIDR/private IPv4。
8. initialize后无监听/worker/LAN secret；start仍一个实例；停止清理及discovery不确定失败保持。

前端最低测试：
1. clean草稿收到新revision自动更新；dirty保留并标冲突；冲突零盲重试；reload/放弃/比较流程。
2. 三值展示：dirty draft / saved effective+source / current real；unloaded selected有历史恢复值但没有当前驻留。
3. 保存profile不load；保存并reload分两步，保存成功后load失败不可假回滚；busy明确保存成功待应用。
4. 临时覆盖与profile覆盖分开；新普通load不全量回传当前显示值；恢复继承发null。
5. migration差异和选择明确；未初始化可离线准备；pending restart不能显示“已生效”。
6. 请求默认单独保存不触发重载/停服，clean请求表单同步新默认、明确临时请求值仍优先；页面导航不会撤销已确认service-start→load链；任务唯一ID，短测失败/延期与load成功分开。

Rust逻辑/React mock/Linux真实GGUF/Windows原生/目标Win10与两机LAN验收分别记录。未接上cold API、只在TS持久化profile或仅编译过均不得称“UI/API统一完成”。

## 10. 契约附件与验证范围

- [JSON Schema](../product/configuration.schema.json)：覆盖新线协议的结构约束
- [请求与响应示例](../product/wire-examples.json)：含在线请求默认更新等合法示例
- Schema不能替代Rust语义校验：batch与context关系、元数据上限、回环/私有地址、CIDR重叠、档案ID唯一性、私有权限、CAS与锁仍由后端强制
- 本决策为实施合同；真正代码、并发、迁移、Windows及发行结果另见验证记录，不将结构校验视为产品验收
