# model-store（T02）

仅离线导入与管理已复制的 GGUF。无 llama 链接、模型下载、Android SDK 或网络访问。`Read + 精确字节数` 的输入适配未来 ContentResolver；不把 `content://` 当文件路径。

## 公共入口

- `ModelStore::open(data_dir)`：取得整个存储实例生命周期的进程独占锁，恢复本模块临时文件，对现有注册项执行完整 hash/结构校验
- `import_file(source, request, cancellation)` / `import_reader(reader, size, request, cancellation)`：复制来源；`ImportRequest::new(id, name, source)` 默认 context 2048，可设置预期 SHA-256
- `list/get`：读取 manifest 派生索引，校验 schema、受控路径和文件大小；不在列表操作中重算所有模型 hash
- `resolve`：已验证缓存 + 有界 manifest / 文件 fingerprint 检查；导入或重验占用写锁时立即返回 `RuntimeBusy`，不等待大文件操作，也不在调度 actor 内重新 hash
- `verify`：显式完整 hash 与 GGUF 重验，刷新缓存；适合阻塞执行器。`open` 与导入同样是阻塞操作
- `remove`：只删除受控注册副本；调用方应先卸载并协调执行器使用，返回路径不等于持有模型租约

`ImportCancellation` 可跨线程设置；每个复制块及提交前检查。外部流的阻塞 `Read` 必须由流适配器自行提供返回机制，本模块不能打断任意调用方的阻塞读取。文件系统同步也不能被安全强杀。

## 一致性与恢复

1. 检查来源可读、ID、可用空间（模型大小外保留 1 MiB）
2. 创建独占 `imports/import-<uuid>.partial`，流式复制与 SHA-256
3. 有界 GGUF metadata / tensor 描述符与数据范围检查；验证 manifest
4. 在 imports 的 `.staged` 目录放入 `model.gguf` 与已 flush 的 `manifest.json`
5. 同一文件系统目录 rename 到 `models/<id>`，两个文件一起成为注册项

不写独立 `index.json`：index 从完整注册目录派生，因此不存在两个文件各自提交产生的索引/manifest分歧。协作进程遵守 `runtime/model-store.lock`；同实例修改互斥，默认拒绝任何已有目标（包括空目录、符号链接和不完整目录）。`.partial/.staged` 出错或取消时清理；中断残留只按本模块 UUID 命名空间恢复，不删除未知临时名称。删除先把注册目录原子移动为 `.deleted`，再清理，重启可续清。

Unix 同步文件与可 fsync 的目录；Windows 文件在 rename 前同步，但标准库没有可移植目录 flush，本阶段只声明原子可见性，不声明断电耐久性。提交后的 sync/cleanup 失败明确说明注册或注销已发生；不能把这类错误理解为事务从未提交。网络文件系统、跨挂载 imports/models、非原子 rename 文件系统不属于当前支持条件。

## 安全与验证边界

- 共享 ID 语法为 `[a-z0-9][a-z0-9._-]{0,63}`；存储额外拒绝尾点与 Windows 设备名（含扩展名），避免跨平台路径歧义
- 数据根目录及受控子目录/文件拒绝符号链接；内部文件系统操作使用 cap-std 目录能力约束，拒绝 `relative_file` 偏离 `model.gguf`
- 数据目录必须是应用私有目录。进程锁协调正常客户端；不是隔离同 UID 恶意写入者的安全边界，fingerprint 不是抵御伪造时间戳的认证机制，也不宣称跨本地恶意并发替换不存在 TOCTOU
- 原始来源只读复制，不移动、不覆盖、不删除；默认错误文本不暴露完整用户路径。T04对源打开采用Unix O_NOFOLLOW|O_NONBLOCK或Windows OPEN_REPARSE_POINT并后验拒绝reparse/非普通文件，避免特殊文件打开竞态阻塞；HTTP仅显式本地普通文件
- 解析限制：64 MiB header、单字符串 ≤ 1 MiB、metadata/tensors 各 ≤ 100,000、数组 ≤ 1,000,000；拒绝嵌套数组、重复键/张量名、非法 UTF-8、超界/溢出/重叠/截断张量
- 当前结构解析支持 F32/F16/BF16、常用 Q4/Q5/Q8 与 K-quants、整数和 F64 tensor 布局；未知 GGML tensor 布局保守拒绝，不随意推断字节长度
- GGUF 结构通过仅表示可安全登记，不表示可推理。只有固定模型矩阵的 hash、大小、qwen3、Q8_0、模板 hash、40960 原生 context 和 2048 默认 context 全部匹配，才记录当前精确 llama commit 与验收证据
- 已验证记录明确只对应 Windows Server 2022 x64 CPU CI / 2 threads / context 2048；`ResolvedModel.context_limit` 对该组合限制为 2048。并不据此声明 Android、其他设备或任意线程数已验收
- 缺少关键字段或矛盾的 validated/capabilities/commit 声明拒绝；普通未验收导入的 `validated=false`，不能通过用户自填字段升级成已验收
- schema/来源中的扩展 JSON 字段可保留，保留字段不能被扩展覆盖。调用层负责日志/导出时的来源隐私处理

## 验证入口

`cargo test --locked -p model-store` 与 `cargo clippy --locked -p model-store --all-targets -- -D warnings`。本 crate 测试仅使用合成 GGUF，覆盖存储、边界、完整性、取消、原子可见性、并发重名、进程锁和恢复，不伪装真实模型推理。真实跨模块测试由 `engine-host/tests/real_runtime.rs` 所属执行器验证提供。
