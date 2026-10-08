# Nexa OCR 插件内部接口

宿主固定研究基线：PI-Desktop 779e16d9c3ca2e966a7ae3db9dd0707243a2831f（0.17.0）。独立 `.piplug`，无宿主/后端补丁。main 由 Node 进程执行，renderer 只用 `window.pluginBridge.invoke`。每个调用快速返回；推理在 main 后台运行，UI 单飞轮询 `ocr.snapshot`（运行时 500–750ms，空闲时 2s）。一个插件实例共享一个队列。

## Settings

`{baseUrl:'http://127.0.0.1:18080', modelId:'', prompt:'Text Recognition:', maxTokens:4096, timeoutSeconds:1800, maxImageEdge:0, view:'markdown', autoLoad:true, rememberToken:false}`。

`maxImageEdge` 为0（原图）或512..8192整数；`timeoutSeconds`为30..86400；maxTokens为1..4096（后端当前上限）。凭据不放进settings/snapshot/history，`ocr.settings`中单独提供token（64位小写hex）。主进程只有显式rememberToken=true才写入插件私有credential文件，其他偏好TOML、history TOML；UI用`hasToken`布尔状态，不回显已保存的令牌。可用password输入框或用户主动选择Nexa的api-token文件导入，不扫描或后台读取用户凭据目录。

## Bridge channels

- `ocr.snapshot({})` -> Snapshot（下表）
- `ocr.settings({patch,token?})` -> Snapshot。运行期间拒绝修改；空token表示保持，清除用`ocr.clearToken`。
- `ocr.clearToken({})` -> Snapshot。
- `ocr.connect({})` -> `{started:true}`，后台连接后snapshot更新；需先保存设置。
- `ocr.image.begin({name,width,height,bytes,mimeType,dataLength})` -> `{uploadId}`。PNG/JPEG、≤4MiB、8192边、≤16777216像素；一次一个暂存上传，dataLength≤5600000。
- `ocr.image.chunk({uploadId,chunk})` -> `{received}`。chunk是dataURL字符串片段，≤196608字符。
- `ocr.image.commit({uploadId})` -> `{id}`。main完整核对dataURL/尺寸后入队，状态pending。最多20图，默认导入顺序。
- `ocr.image.abort({uploadId})` -> `{ok:true}`。
- `ocr.image.read({id,offset})` -> `{chunk,total}`，每次最多196608字符；可供新打开窗口恢复预览。
- `ocr.queue({action:'move',id,direction:-1|1})` / `{action:'remove',id}` / `{action:'clear'}` -> Snapshot。运行中拒绝变动；已开始图不能改回pending或自动重放。
- `ocr.start({})` -> `{started:true}`，后台按顺序处理pending，复用模型。停止/失败暂停整批，再开始仅继续pending，不重做已开始项。
- `ocr.stop({})` -> `{stopping:true}`，终止本插件当前加载/识别并保存部分文字；直到清理确认前保持忙。
- `ocr.result({id})` -> Result|null，可查队列已运行项/历史。
- `ocr.history.delete({id})` / `ocr.history.clear({})` -> Snapshot（运行中拒绝）。
- 导出用renderer Blob + `<a download>`（需实际Electron验证），或`ocr.export({id})`通过主进程 `pi.fs.requestDirectory`+`pi.fs.writeText` 写用户选择目录，不允许UI提供任意磁盘路径。

复制使用`pluginBridge.invoke('clipboard.writeText', {text})`（按上游实际约定可调整）。

## Snapshot

```ts
{
 settings: Settings, hasToken: boolean,
 connection: {state:'disconnected'|'connecting'|'connected'|'error', message:string, instanceId:string|null},
 models: Array<{id:string, name:string, hasProjector:boolean, loadable:boolean}>,
 runtime: object|null,
 busy:boolean, stopping:boolean, phase:string, error:string|null,
 queue: Array<{id:string,name:string,width:number,height:number,bytes:number,status:'pending'|'running'|'completed'|'failed'|'cancelled',error:string|null}>,
 activeId:string|null,
 result: Result|null,
 history: Array<{id:string,name:string,modelId:string,createdAt:string,status:string,complete:boolean,preview:string}>,
 persistenceError:string|null
}
type Result = {
 id:string,name:string,modelId:string,createdAt:string,
 status:'running'|'completed'|'failed'|'cancelled',
 text:string,complete:boolean,error:string|null,requestId:string|null,
 finishReason:string|null,usage:object|null,performance:object|null, elapsedMs:number
}
```

性能使用Nexa原始PerformanceRecord（performance.timings含prepare_us/prefill_us/decode_us/output_callback_us，外层timings为queue_ms/load_ms/execution_ms），不把页面计时冒充引擎指标。无数据写null。只有匹配实例+request ID的记录可附加。历史只保存最多100条非空结果，图片/令牌不保存。文本总上限单条1MiB，历史总上限16MiB；写入失败要提示并暂停批次，不能假称已保存。

`ocr.export`快速返回`{started:true}`，主进程后台弹目录窗口、写入新名字的Markdown，结果放snapshot的`exporting:boolean`与`exportMessage:string|null`。

## 生命周期

首版运行按钮属于本插件，直接调用Nexa管理/推理接口，不进入PI Agent工具历史。同模型复用，空闲异模型显式切换，忙/故障明确提示。不自动下载模型、不终止别的客户端任务、不部分输出后重试、不放宽Nexa认证/Origin规则。卸载插件时Abort并保存当前部分结果。
