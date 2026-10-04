# ADR0016：混合模型目录的部分登记与有界诊断

日期：2026-10-03。状态：实现已冻结，本机合成回归与独立源码/事务审查通过；精确提交WindowsCI、产品包及用户目标机待验证。基线为`f3e1b90`，已发送产品仍为`50c9d41`。本决策规定本轮契约，已实施范围与实际检查见末节，不追溯修改旧包行为或既有验证证据。

## 背景

[ADR0015](0015-open-model-loading-and-validation-evidence.md)已将开放候选加载与历史validated分开；50c9d41允许尝试符合结构/安全/文本契约的单文件GGUF，但目录内一个坏文件仍使整批登记失败，metadata context小于默认2048的模型也无法按扫描默认值登记。用户要求广泛模型支持，目录管理不应把可明确隔离的内容问题升级成其他合法文件无法使用。

范围仅为model-store扫描、私有desktop bridge DTO和桌面展示，不改公共HTTP、worker IPC、native行为身份、library schema或managed显式导入语义；不新增工具能力，不放宽GGUF解析、安全或资源门槛。

## 决策

### 1. 合法集合一次发布

完整枚举、核验和诊断先在内存完成。至少存在一个合法候选时，合法集合一次原子替换external索引；它不是边扫描边保存，也不是把旧索引与新扫描随意拼接。旧managed记录和源GGUF均不变。

| 完整扫描结果 | 终态/发布 | 目录与generation |
| --- | --- | --- |
| 所有候选合法 | `completed`，发布全部合法集合 | 一次新generation |
| 合法与内容拒绝并存 | `partial`，只发布合法集合，并返回全部被拒文件的有界诊断 | 一次新generation；不得称完整成功 |
| 有候选但全部内容拒绝 | `failed` / `model_scan_no_usable_files`，不发布 | 保留原目录、索引和generation |
| 无GGUF候选且完整枚举成功 | `completed`，允许发布空external集合 | 一次新generation；不同于全坏目录 |
| 任一硬失败、取消或提交前超时 | 失败/取消，不发布 | 保留旧事实；已发布后的持久性异常另按既有规则处理 |

scan-only必须核对原选目录的OS身份，路径相同不代表原目录仍在；显式apply可选择新目录。稳定ID沿用同一目录身份、同一basename和同一hash规则；成功partial会移除未进入新合法集合的旧external注册，不保留幽灵条目。

### 2. 内容拒绝与硬失败必须有类型边界

每个文件必须先经过受保护读取、真实I/O错误观察、取消/deadline与读取前后身份核对，才可把确定的内容问题归为逐文件拒绝：`invalid_manifest`、`unsupported_model`、`unsupported_chat_template`。不能仅凭一个宽泛错误字符串跳过文件。

所有既有预算继续是硬失败，包括目录条目、候选数、单文件与总字节、路径/文件名、GGUF header/string/metadata/tensor/数组、索引、诊断及完整operation字节预算。坏文件仍计入候选数和字节预算；不得先拒绝再从总量扣除。目录或文件I/O、身份变化、路径/reparse/非普通文件、取消、超时及保存失败同样不得降为软拒。

GGUF解析为扫描提供私有typed预算错误路线`read_for_scan`，预算触限映射现有`ModelLibraryLimit`；既有managed read/导入的错误语义保持。对于提前硬失败，只展示已经观察到的诊断并明确整次未发布，不能宣称其余未读取文件均已检查。

### 3. guard与提交决策

成功文件和软拒文件的只读句柄、目录身份guard在继续扫描期间都保留，覆盖最终提交或全坏放弃决定。软拒不能提前放开文件后继续得出整库稳定结论。

硬失败确定本轮不可发布并退出扫描后，可释放其guard；不要求为等待外层UI `finish` 或poll继续占有资源。实例锁和blocking扫描的结束仍按既有关闭规则协调；terminal在工作/实例锁释放后发布，避免前端一读到终态立即重扫却误报busy。不能提前宣称已取消或服务可启动。

取消先赢则不发布；明确提交决策先赢则完成原子发布，晚到取消不伪称回滚。rename后fsync失败继续返回`settings_durability_unconfirmed`并读取真实generation，不能报告旧索引必然保留。

### 4. 私有DTO与诊断生命周期

`model_library_next`新增status=`partial`、`file_errors:[{file_name,code,message}]`，result新增`rejected_files`。旧服务的`completed`结果缺少两项字段时按空列表/0处理；新旧任意混搭不因此自动取得完整兼容保证。`partial`必须含真实提交result与完整诊断，不能没有结果却当成功。

诊断仅含经校验basename和受控code/message；不含完整路径、token、原始native/parser异常或模型内容，不写日志/CI。诊断不进入model-library.json或其他持久化历史，仅保留当前App生命周期内的有限operation状态。现有一个活动操作/一份有界终态和1Hz不重叠poll保持。

完整诊断序列化预算≤512KiB，完整operation DTO≤1MiB；超界为硬失败，不截断诊断后宣称partial成功。单basename≤1024 UTF-8字节，数量同时受64候选限制；前端也验证响应上限和字段一致性。

### 5. 仅扫描选择短context默认值

自动目录扫描用`default_context=min(2048, metadata.context_length)`建立合法候选，仍要求metadata与manifest本身有效。显式import/load参数、runtime/桌面设置不变，不静默夹紧用户指定值；登记成功不表示当前UI加载参数适合该模型，更不证明16GB可用内存或真实推理成功。

### 6. 产品包边界不变

包内preflight继续严格执行声明文件、PE/许可/hash以及受支持模型目录规则。混合坏文件的功能验收使用包外授权目录，不能为展示逐文件诊断而允许不合规包内文件通过产品校验。旧50c9d41不因本文获得partial或短context扫描能力。

## 验证与后续

验收至少覆盖好+坏、全坏、空目录、旧generation/ID、硬限额（含parser预算）、I/O/身份/reparse、取消/超时/保存、soft-reject guard、scan-only目录替换、低context及显式参数不降级、旧DTO默认与UI完整诊断。Linux逻辑、Windows共享访问、真实GGUF、原生窗口和用户目标机各自记实；测试结果见[本轮记录](../verification/2026-10-03-mixed-model-directory.md)。最终主代理完整workspace聚合343 pass/0 fail/7 ignored、完整clippy，以及UI85项/typecheck/lint/build通过（此前8crate266/0/1不另加总），独立源码/事务审查无未解阻断。Windows特定guard、真实模型/整包/原生窗口、磁盘write/fsync故障注入未因此通过；尚未执行的项仍保留待验证。
