# T06 Windows 最小桌面契约

2026-10-01。实施定界，不是完成报告。以执行规格8.2为基础；T05已有固定Windows Release CI与独立Windows 10手工短验，无开发工具、离线及长期稳定性仍待后期验证。Windows 10优先，Windows 11后续，不更换Tauri 2 + React/TypeScript/Vite，不扩Telegram或runtime职责；2026-10-03模型范围按[ADR0015](decisions/0015-open-model-loading-and-validation-evidence.md)更新为开放候选，源码/本地回归已完成，Windows与新包窗口验收待完成，不追溯既有包。

## 1. 三个工位与唯一写入范围

1. **bridge**：`crates/desktop-bridge/**`、根 `Cargo.toml` / `Cargo.lock`。纯 Rust，依赖现有 `runtime-cli` / `runtime-api` / `runtime-types`；不依赖 Tauri、engine-host、llama-adapter。负责安全客户端、生命周期、设置存储、SSE、有界流和可在 Linux/Windows 运行的宿主集成测试。
2. **UI**：`apps/desktop/` 下的前端源代码、HTML/CSS、TypeScript/Vite/测试配置、`package.json` / `package-lock.json`；不写 `src-tauri/`。按本契约封装单一 `invoke` 适配层；mock 仅在组件测试或显式开发预览使用，发行构建不能自动回退为 mock。
3. **shell/CI/package**：`apps/desktop/src-tauri/**`、桌面打包脚本/模板、`.github/workflows/native-windows.yml`、必要 `.gitignore` 与验证/状态文档。负责原生对话框/剪贴板/窗口关闭，调用 bridge，Windows 真实 Tauri 构建、解压包验证。根 manifest/lock 改动必须交 bridge 工位。

父级负责整合与最终提交。共享构建目录和锁文件操作串行协调；子代理不提交、不再派生。本文如需改变共享签名，先通知另外两工位，不私自两边各写一套。

## 2. 构建隔离

- 根 workspace 新增 `desktop-bridge`，显式 exclude `apps/desktop/src-tauri`。原有全 workspace 检查新增纯 Rust bridge，但不引入 WebView 系统依赖。
- `apps/desktop/src-tauri/Cargo.toml` 自带 `[workspace]`，独立 `Cargo.lock`，通过 path 依赖根 bridge。不能给这个独立 workspace 使用根的 `workspace = true` 元数据。
- 两个 workspace 不能真实共用一个 Cargo.lock。桌面锁可用根锁作为初始解析基础，再锁定新增 Tauri 图；记录共享依赖的实际解析差异，分别 `--locked`，不链接/覆盖根锁。两个图都检查无 native 推理链接。
- 前端锁独立。已有 Node 24.19.0 / npm 11.9.0 可作为待验证组合，实际构建通过后写构建锁；不得先称 Tauri/React/Vite 的任意版本已锁定可用。
- Linux 首先验证 bridge 和前端；Windows CI 单独构建 Tauri 壳。bridge 真实 HTTP 测试复用现有真实模型与临时凭据。Windows 壳检查不能被 Linux 浏览器预览或 Rust bridge 测试替代。

## 3. 最小产品范围

- 模型：区分历史validated、可尝试loadable与当前available，未实测合法候选不因型号/hash缺少批准而禁用；旧服务缺loadable要求匹配版本。实际加载/模板/资源失败明确提示，目录仍可能因单个坏文件整批失败。原生选择本地 GGUF、显示文件名/大小和复制目标、输入模型 ID、导入、列表/分页、加载/卸载、状态与错误。API 无字节进度，显示真实“导入中”忙碌指示，不制造百分比。
- 聊天：当前会话消息、发送、批量流式文本、停止、清空。正文作为纯文本渲染；不执行 HTML、链接、模型指令。没有持久化聊天。
- 设置：CPU、上下文、线程/批次、默认输出预算、空闲卸载、关闭时同时退出开关、只读本机 API 地址和原生复制令牌按钮。
- 新 UI 偏好可采用已验预设 `cpu / context2048 / threads2 / batch128 / max_output512`；清楚显示为 UI 的下一次加载/发送参数。不得悄悄覆盖既有 runtime 配置/已加载参数，也不把配置值称为原生实测后端。
- 无模型、初始化缺失、连接失败、正在加载/导入/停止、Faulted、上下文超限都有明确恢复操作。`backend=null` / `unavailable` 原样表达，不能伪报实测 CPU 或内存值。

## 4. Rust ↔ 前端共享接口

命令为下列 snake_case 名；所有业务 DTO 字段也为 snake_case。有参数时统一 `invoke(name, { request: ... })`。只暴露固定用途命令，不提供任意 URL、HTTP 路径、进程命令、文件读写或 token getter。

- `desktop_snapshot()` → `{ initialized, connection, api_address, runtime, settings, model_directory }`。`connection` 为 `stopped | connecting | connected | error`；runtime 为现有安全 status DTO 或 null。`settings` 含 `context_size, threads, batch_size, max_output_tokens, idle_unload_seconds, close_runtime_on_exit`；CPU 固定。`model_directory`区分配置目录与同TCP proof后服务实际采用的目录，不用本地配置伪称已生效。错误使用受控 `code/message`，不回传 token、完整原始异常或请求正文。
- `runtime_start({ initialize_if_missing: boolean })` → snapshot。首次初始化必须来自用户显式“初始化并启动”动作，复用幂等 init，不轮换既有 token。自动启动已初始化实例可复用此入口并传 false。
- `model_pick()` → null 或 `{ selection_id, file_name, size_bytes, destination }`。原生壳保存选中路径，前端只传一次性选择 ID；取消选择不改变当前模型。`destination` 是供用户确认的管理目录显示文本。
- `model_import({ selection_id, model_id })` → 安全 ModelSummary。壳将受控路径交 bridge，后者走现有 `/runtime/models/import`；同一选择不能并发提交。失败时不自动重试，特别是 `import_committed_durability_unconfirmed` 必须先刷新模型列表。
- `models_page({ after: string|null, generation: UUID|null })` → `{ generation, data: ModelSummary[], next_after: string|null }`。第一页两个参数均null；后续携带generation，陈旧页拒绝并从第一页重新取。固定页大小64，前端只保留当前页。ModelSummary增加managed/external来源与安全不可用原因，显示名为主；选中模型显示名来自实际服务，不依赖当前页。
- 新桌面主流程使用`model_directory_pick()`、`model_directory_apply({selection_id})`、`models_scan()`、`model_library_next({operation_id})`、`model_library_cancel({operation_id})`五个固定命令，完整DTO/预算/取消与停止条件见[外部目录契约](t06-model-directory-contract.md)。原生目录选择允许支持的本地可读目录，不写源目录；JS只有一次性选择ID与只读展示路径，绝不提交自由路径。仅bridge真正接纳apply后消费ID，busy拒绝保留选择。新流程自动命名/生成内部ID、不复制已有GGUF，原model_pick/import及公开复制API只保留兼容。
- `model_load({ model_id, context_size, threads, batch_size })` → runtime status，固定 `backend=cpu,gpu_layers=0`。已加载参数与UI参数不同必须明确展示；不静默换模型。16GB总内存不是可用量或模型容量保证，metadata/131072硬限也不是建议值；不宣称已有Job RAM硬限。`model_unload()` → runtime status。
- `chat_start({ model_id, messages, max_output_tokens })` → `{ request_id }`。messages 只含受控 role/content。bridge 原子占用本 UI 唯一生成槽，先创建并登记 UUID，然后返回；后台发送 HTTP，并在同一请求带 `X-Request-ID`。重复发送返回 `desktop_busy`，不排出第二条本 UI 生成任务。模型加载通过独立 model_load 完成，UI 未就绪时不误称已开始生成。
- `chat_next({ request_id })` → `{ request_id, events, terminal }`。单一未完成消费者、长轮询至事件或最多 1 秒心跳；顺序与上次连续。event 为 `{ type:"started" }`、`{ type:"delta", text }`、`{ type:"completed", finish_reason, usage }`、`{ type:"cancelled" }` 或 `{ type:"failed", code, message }`。终态后保留一个小型终态记录，重复读取返回同一终态摘要、无重复文本。
- `chat_cancel({ request_id })` → `{ request_id, status:"stopping" }`。只准取消本 UI 持有的 ID；真正终态由 chat_next 返回。取消不占用生成/导入工作锁。
- `settings_save({ settings })` → snapshot。参数只含 `context_size, threads, batch_size, max_output_tokens, close_runtime_on_exit`，校验后在独立 `desktop-settings.json` 单文件原子保存，运行中也可保存UI默认值；不改runtime配置，不把UI偏好写进runtime配置注释。
- `runtime_idle_save({ idle_unload_seconds })` → snapshot。前端单独提供“应用空闲卸载设置”动作；仅服务已停止且持实例锁时校验并原子更新runtime config，下次启动生效。运行中返回 `runtime_running`，不自动关停其他客户端、不伪称热更新。snapshot.settings中的idle值来自runtime config，其他UI偏好来自独立文件；两次显式动作各自只写一个文件，不承诺跨文件事务。
- `token_copy()` → `{ copied:true }`，只由用户按钮触发。原生壳通过现有私有文件校验读取，直接写原生剪贴板；前端不收到 token，也没有剪贴板读取能力。按钮提示令牌会进入系统剪贴板。
- `runtime_stop()` → `{ stopped:true }`，仅在真实 HTTP shutdown 成功且 `wait_stopped` 确认原实例锁/记录释放后返回。该操作会停止全部客户端任务，UI 在入口明确告知。
- `desktop_close()` → void。与原生窗口关闭事件调用同一关闭状态机，不绕过本 UI 请求清理及关闭设置。

状态刷新最多 1Hz且不重叠；动作后可刷新一次，不周期读取日志。前端界面重绘/文本批处理最多约30Hz。UI 工位定义上述 TS DTO，bridge 工位对应 Rust serde DTO；共同使用本文字段，不因 camelCase 自动转换假设造成偏差。

## 5. 本机身份与生命周期

1. 数据目录沿用 CLI 的 `%LOCALAPPDATA%/Nexa`（测试显式临时目录），不创建另一套隐含服务。配置/令牌权限校验复用已有代码。
2. 发现文件、PID和端口只作定位。每条新 TCP 必须使用 `runtime_cli::client::VerifiedConnection` / `connect_data_dir` 完成 endpoint-bound HMAC proof，再在同一连接发送 Bearer；禁止 reqwest 池化、代理、重定向、自动重连/重放。不同控制请求可各建独立已证明连接，避免取消排在生成后面。
3. 已占实例锁但连接/proof失败时显示错误，绝不据此抢锁、删记录、杀PID或另启替身。锁空闲时从固定随包路径启动；启动竞态由已有实例锁和端口绑定裁决，失败后只重新发现/校验，不自动重放业务请求。
4. 壳从已验证产品布局解析 `runtime/ai-runtime.exe`，它与 `ai-runtime-worker.exe`及CRT保持同目录。Rust 原生 Command 固定参数，无 shell/PATH/CWD搜索。Windows固定使用`DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`，不请求`CREATE_BREAKAWAY_FROM_JOB`，不在失败后改变策略重试。Nexa不创建在UI退出时杀掉runtime的sidecar、kill-on-drop或UI-owned Job；stdin/stdout/stderr不依赖UI存活的管道，输出不无限收集。尊重外部宿主既有Job/会话containment，不修改其限制/权限，不保证整个外部Job或会话终止后runtime仍存活，也不尝试脱离该限制。runtime对子worker的既有Job回收规则不变。
5. 默认关闭：先停止接收UI操作，取消本UI在途请求，关闭本UI HTTP流并有限等待清理；正常关闭保留 runtime 及其他客户端请求。UI消失/崩溃造成流断开也要触发现有断流取消，不自动重放。
6. 选择同时退出或显式停止：明确说明影响其他客户端，走既有 shutdown + wait_stopped；未确认清理时不得显示已停止。原生关闭事件先阻止立即退出，运行同一异步流程，重复关闭合并；失败给保留窗口/重试/仅关UI的明确选择，不猜PID强杀。
7. Tauri只允许main本地打包窗口调用固定命令，禁远程导航/远程资源和通用shell/HTTP/fs/clipboard-read权限。配置CSP；自定义命令要通过 AppManifest::commands + capabilities 精确授权，不能误以为注册命令默认受插件ACL限制。

## 6. 有界流、取消竞态与历史

- bridge只允许1条本UI在途生成，最多1个未完成chat_next；多次/跨ID消费拒绝。SSE解码缓冲上限64KiB，单事件上限32KiB，拒绝超限/无效UTF-8/异常JSON，不无界寻找分隔符。现有服务单delta编码上限约25KiB，故该上限覆盖正常事件。
- bridge待消费文本队列总上限64KiB，元事件数量上限32；单次chat_next返回UTF-8文本累计≤16KiB，控制事件受独立小型预算限制。队列满则暂停读HTTP，自然反压；10秒没有前端消费进展则终结本UI流、请求取消，不能持续堆积到Tauri或WebView。前端最多1个批次在途，不预取无限批次。
- 每次回复累计UTF-8文本上限256KiB；当前会话累计正文上限512KiB、最多128条消息（都包含当前回复）。UI提交chat_start时的完整序列化JSON≤512KiB。命中任何上限时明确报错并取消；保留标为“不完整”的已有文本，绝不静默截断或把部分文本冒充成功。下次发送要求用户显式清空或删减，不静默丢弃最旧历史。
- 前端只存一份已合并会话文本，不保存每个token/delta副本。清空在生成中先取消，确认本UI终态后清空；不调用全局shutdown、不取消其他客户端、不影响其历史。
- RequestID在任何网络发送前登记；取消先于发送时本地终结且绝不发送；发送中/送出后取消必须关闭该HTTP流并按必要发送独立已证明cancel请求。取消发生在服务接纳前可能得到404，不能把404当作生成成功或取消已确认；结合本地发送阶段、连接关闭和实际流终态收口。取消意图不可因started响应晚到而丢失。
- `202 cancelling`不等于完成。成功只能来自合法finish后`[DONE]`；include_usage=true，保留真实usage。SSE error、显式取消、突然EOF/损坏帧都是非成功终态；不补造[DONE]、usage或重放。终态每次内部只产生一次。
- 终态错误/取消后的部分assistant消息明确标注，不作为完整回复静默复用。UI可保留用于查看，但再次发送前要显式清空或移除该未完成轮次。

## 7. 首包与验证门槛

- 私有开发 `desktop-windows` ZIP：Tauri EXE/嵌入前端 + `runtime/`下完整匹配T05产品包及其manifest/许可；额外桌面依赖按实际PE闭包补齐。模型仍外部导入，不复制用户数据/token；UI与runtime大小分开统计。
- 发行 ZIP 继续严格无模型、文件集合与 manifest/SHA256SUMS 一致。用户解压后的程序根目录，以及固定`model/`、`models/`目录允许直接普通`.gguf`输入，扩展名大小写不敏感、只读四字节GGUF头、保持symlink/reparse拒绝；不hash整个输入、不计产品清单或体积、不自动导入。两个固定目录可为空，不允许未知文件或嵌套目录；runtime/licenses等其他位置、未声明EXE/DLL/脚本继续严格拒绝，全部声明文件hash/来源校验不变。原生选择其他包内目录时立即明确拒绝，不等重启；选包内允许目录时在pick/apply/重扫前复用layout校验，以拒绝UI打开后新增的不合法文件。重扫validator使用bridge持实例锁时确定的实际目录/索引，不能从display_path读取授权。包外任意受支持目录不属于产品库存。包校验不读取用户配置决定豁免，四字节识别不代替注册/加载的完整核验。
- 启动包校验保留固定错误码到原生中文提示与 `--diagnose` schema 2；不回显用户绝对路径或任意错误文本。CI wrapper 与 evidence stager 对成功/失败报告执行同一闭合字段/错误码白名单，非零退出的合法失败报告也保存。
- 本轮用已安装的Evergreen WebView2，不自动下载、安装或改变系统权限。缺失时在WebView建立前给原生可读错误及微软官方安装入口；不能只做网页内错误（网页根本无法启动）。检测实际版本并纳入验证证据，Windows 10或装有Edge都不等于WebView2一定存在。
- 不新增fixed WebView2；微软当前文档说明Win10非打包Win32使用fixed v120+涉及AppContainer目录ACL要求，不适合本轮未经批准的系统权限变化。
- 自动验证：根回归；bridge假服务恶意proof/redirect/丢帧/大帧/取消竞态/慢消费者/终态一次；Linux真实模型import→load→流→cancel→再次生成→unload→stop；Windows相同宿主链及实际Tauri构建/PE闭包/中文空格路径。
- 既有 Windows 桌面解压验收步骤另用实际 EXE 检查：同级真实 GGUF 输入允许；未声明 DLL 与改动 manifest 均拒绝；只清理测试独占创建的输入，恢复清单原字节后重新严格核验产品 payload。此项不建立窗口、不替代独立原生UI验收。
- Windows关闭语义须用真实子进程验证：默认关UI后必须证明同实例API仍可调用；同时退出后实例锁/记录释放、worker回收。正常关闭自身UI与外部宿主终止整个Job/会话分开，不以启动探针代替完整生命周期证据。关闭中重复点击、连接既有runtime、异协议/proof失败、端口占用、取消在started前后均覆盖。bridge harness不能冒充已验证原生窗口事件；壳若暂不能自动驱动，明确保留Windows 10独立手工操作验收。
- 前端单测/浏览器检查三页、键盘/中文输入、无模型/忙碌/失败/清空/重复点击/切页；mock只证明UI。T06完成仍需Windows UI实际导入、真实聊天、停止和关闭两种语义，不用截图代替生成。

## 8. 官方核对来源

- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)：工作区成员共享本工作区根锁；独立工作区分开管理。
- [Tauri capabilities](https://v2.tauri.app/security/capabilities/)：本地窗口能力及自定义命令默认行为。
- [Tauri Rust→前端](https://v2.tauri.app/develop/calling-frontend/)：Channel提供有序事件，但不替代应用自己的有界消费设计；本轮采用单消费者pull。
- [Tauri外部二进制](https://v2.tauri.app/develop/sidecar/)：externalBin/架构命名约定；本轮可用原生Command维持既有产品目录和独立生命周期。
- [Tauri Windows发行](https://v2.tauri.app/distribute/windows-installer/)与[微软WebView2分发](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)：WebView2实际检测、Evergreen/Fixed分发及Win10限制。目标环境需实测，不从文档平台列表推导验收通过。

当前没有必须先让用户决定的新产品分叉。WebView2存在性、Windows壳实际行为和依赖精确版本属于待验证工程事实；无开发工具/离线/长期稳定性与Win11继续分层保留，不阻塞已授权T06开发。
