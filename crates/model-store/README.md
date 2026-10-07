# model-store（T02）

管理managed复制导入与external只读零复制GGUF目录，无llama链接或网络访问；本轮新增仅文件侧DownloadFile事务，由desktop-bridge显式下载器供字节，不承担网络请求。`Read + 精确字节数`保留通用有界输入能力，不把URI当本地文件路径。

## 公共入口

- `ModelStore::open(data_dir)`：取得整个存储实例生命周期的进程独占锁，恢复本模块临时文件，对现有注册项执行完整 hash/结构校验
- `import_file(source, request, cancellation)` / `import_reader(reader, size, request, cancellation)`：复制来源；`ImportRequest::new(id, name, source)` 默认 context 2048，可设置预期 SHA-256
- `list/get`：读取 manifest 派生索引，校验 schema、受控路径和文件大小；不在列表操作中重算所有模型 hash
- `resolve`：已验证缓存 + 有界 manifest / 文件 fingerprint 检查；导入或重验占用写锁时立即返回 `RuntimeBusy`，不等待大文件操作，也不在调度 actor 内重新 hash
- `verify`：显式完整 hash 与 GGUF 重验，刷新缓存；适合阻塞执行器。`open` 与导入同样是阻塞操作
- `remove`：只删除受控注册副本；调用方应先卸载并协调执行器使用，返回路径不等于持有模型租约
- `unregister` 模块及对应 store 操作：只修改同一 `model-library.json` 的登记可见性，保留外部/受控模型文件、manifest、档案和测试历史；不是调用上述破坏性 `remove`。在线须持有 actor 登记租约，离线须持有停止实例锁和存储锁，详见[ADR0030](../../docs/decisions/0030-nondestructive-model-unregistration.md)

`ImportCancellation` 可跨线程设置；每个复制块及提交前检查。外部流的阻塞 `Read` 必须由流适配器自行提供返回机制，本模块不能打断任意调用方的阻塞读取。文件系统同步也不能被安全强杀。

## 一致性与恢复

1. 检查来源可读、ID、可用空间（模型大小外保留 1 MiB）
2. 创建独占 `imports/import-<uuid>.partial`，流式复制与 SHA-256
3. 有界 GGUF metadata / tensor 描述符与数据范围检查；验证 manifest
4. 在 imports 的 `.staged` 目录放入 `model.gguf` 与已 flush 的 `manifest.json`
5. 同一文件系统目录 rename 到 `models/<id>`，两个文件一起成为注册项

不写独立 `index.json`：受控身份从完整注册目录派生，schema3 library 仅增加登记可见性覆盖层；移除/显式恢复与 external 索引同文件原子提交，不修改受控 manifest。协作进程遵守 `runtime/model-store.lock`；同实例修改互斥，默认拒绝任何已有目标（包括空目录、符号链接和不完整目录）。`.partial/.staged` 出错或取消时清理；中断残留只按本模块 UUID 命名空间恢复，不删除未知临时名称。删除先把注册目录原子移动为 `.deleted`，再清理，重启可续清。

Unix 同步文件与可 fsync 的目录；Windows 文件在 rename 前同步，但标准库没有可移植目录 flush，本阶段只声明原子可见性，不声明断电耐久性。提交后的 sync/cleanup 失败明确说明注册或注销已发生；不能把这类错误理解为事务从未提交。网络文件系统、跨挂载 imports/models、非原子 rename 文件系统不属于当前支持条件。

## 安全与验证边界

- 共享 ID 语法为 `[a-z0-9][a-z0-9._-]{0,63}`；存储额外拒绝尾点与 Windows 设备名（含扩展名），避免跨平台路径歧义
- 数据根目录及受控子目录/文件拒绝符号链接；内部文件系统操作使用 cap-std 目录能力约束，拒绝 `relative_file` 偏离 `model.gguf`
- 数据目录必须是应用私有目录。进程锁协调正常客户端；不是隔离同 UID 恶意写入者的安全边界，fingerprint 不是抵御伪造时间戳的认证机制，也不宣称跨本地恶意并发替换不存在 TOCTOU
- 原始来源只读复制，不移动、不覆盖、不删除；默认错误文本不暴露完整用户路径。T04对源打开采用Unix O_NOFOLLOW|O_NONBLOCK或Windows OPEN_REPARSE_POINT并后验拒绝reparse/非普通文件，避免特殊文件打开竞态阻塞；HTTP仅显式本地普通文件
- 解析限制：64 MiB header、单字符串 ≤ 1 MiB、metadata/tensors 各 ≤ 100,000、数组 ≤ 1,000,000；拒绝嵌套数组、重复键/张量名、非法 UTF-8、超界/溢出/重叠/截断张量
- 模板、metadata key和tensor name中的NUL拒绝，避免C-string截断身份；普通tokenizer metadata value允许NUL，不全局禁字符串NUL
- 当前结构解析支持 F32/F16/BF16、常用 Q4/Q5/Q8 与 K-quants、整数和 F64 tensor 布局；未知 GGML tensor 布局保守拒绝，不随意推断字节长度
- GGUF 结构通过仅表示可安全登记，不表示可推理。只有原矩阵的hash/大小/qwen3/Q8_0/模板hash、40960 metadata context和2048默认context匹配时记录既有精确llama/实测证据；这仅控制validated声明，不再限制其他合法候选尝试加载
- 已验证记录明确只对应 Windows Server 2022 x64 CPU CI / 2 threads / context 2048；历史验证不限制候选请求窗口；`ResolvedModel.context_limit`取模型metadata与131072硬限较小者，实际模板/loader/资源仍须检查。大于2048、其他设备或线程数不因此成为已验收
- 缺少关键字段或矛盾的validated/capabilities/commit声明拒绝；普通未实测导入validated=false，仍可有独立loadable候选资格，不能由用户字段伪造验证。summary.loadable不替代load前文件准备/hash/metadata与external lease
- managed导入前、manifest/load与external统一单文件≤16GiB；是文件安全预算，不是16GB RAM保证。分片、未知tensor结构与缺模板仍拒绝；50c9d41仍保留默认2048扫描对短context的限制；本轮ADR0016仅将自动扫描default_context改取min(2048,metadata)，显式managed import/load参数语义不变，43ad5c2 WindowsCI及包发送完成、用户目标机待验
- schema/来源中的扩展 JSON 字段可保留，保留字段不能被扩展覆盖。调用层负责日志/导出时的来源隐私处理

## 验证入口

`cargo test --locked -p model-store` 与 `cargo clippy --locked -p model-store --all-targets -- -D warnings`。本 crate 测试仅使用合成 GGUF，覆盖存储、边界、完整性、取消、原子可见性、并发重名、进程锁和恢复，不伪装真实模型推理。真实跨模块测试由 `engine-host/tests/real_runtime.rs` 所属执行器验证提供。


## 外部目录（T06）

ModelStore保留现有managed布局，同时从data root的model-library.json读取独立external注册。外部记录显式storage=external，relative_file仅为直接子级安全文件名；managed目录拒绝external标记，复制import的ID与external碰撞会明确拒绝，remove不会删除外部源。显示名默认保留GGUF文件名中的中文与空格，内部ModelId仍严格且由Rust自动生成。

读取旧索引只校验结构及路径语法，拔掉外置盘不会阻止旧managed使用；访问外部源时再只读核查DriveType、祖先/文件reparse和OS身份。首次实际加载重hash并保留Windows source guard到已确认shutdown，actor resolve不hash。外部扫描/读取从不向所选目录写文件；持久化由可信应用在data root持实例锁原子完成。上限与取消/生命周期契约见 [目录契约](../../docs/t06-model-directory-contract.md)。

本轮[ADR0016](../../docs/decisions/0016-mixed-model-directory-diagnostics.md)已由43ad5c2 WindowsCI及包发送验证、用户目标机待验：对受保护读取后明确的内容拒绝形成有界诊断，合法集合仍一次原子发布；全坏不发布，空目录可空提交。坏文件计入全部预算；私有read_for_scan将parser预算typed映射既有ModelLibraryLimit，managed read/导入错误语义保持。I/O、路径/reparse、身份变化、取消/超时与全部限额为硬失败，不能跳过。扫描only在同一次DirectoryGuard核旧目录身份；显式apply可选新目录。成功/软拒source guard保持到提交或放弃决定；确定不可发布的硬失败退出后可释放，不等待UI轮询。诊断不进入schema1索引。实现与验证状态见[本轮记录](../../docs/verification/2026-10-03-mixed-model-directory.md)。

本轮[ADR0017](../../docs/decisions/0017-model-discovery-and-catalog-download.md)增加`library::download::DownloadFile`，仅显式下载可创建受保护UUID.part并在全量size/hash核验后原子hardlink no-clobber发布。模型目录与祖先身份须匹配，已有或竞争目标绝不覆盖；仅清理本任务实际持有的partial对象。生产文件保护仅Windows实现，Linux单元fixture只能验证字节事务；发布后临时项清理未确认须承认文件已保存并上报warning。普通scan/prepare仍只读，不调用该写路径。下载不登记模型、不迁移到managed，也不自动赋validated；使用者须另扫再load，详见[本轮记录](../../docs/verification/2026-10-03-model-catalog-download.md)。

Windows合成共享访问与预存可写mapping测试仅证明各自观察，Linux不能冒称拥有Windows强制共享保证。未确认cleanup的prepared guard有界保留到进程退出，进程随后拒绝重新open catalog；正常调用者必须在worker确认停止后显式release_external_after_shutdown。
