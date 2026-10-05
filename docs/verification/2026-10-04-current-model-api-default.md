# 空模型ID使用当前模型：验证记录

**最新安排：** 用户2026-10-04 13:28 UTC明确恢复提交与GitHub Actions构建。本批四项修复统一在codex/dev交付；下文“暂缓”是此前检查时的状态。源码验证通过不等于Windows原生构建或新包交付成功，后续以精确提交CI结果为准。

日期：2026-10-04。分支：`codex/dev`，用户明确授权在本机和LAN文本API中使用当前加载模型作为空ID默认。契约见[ADR0024](../decisions/0024-current-loaded-model-chat-default.md)。状态：源码实现、最终本地回归与独立审查完成；暂缓Windows打包和Actions。

## 修改边界

- HTTP DTO以可选模型选择保留缺省/空串/全空白，其他字段和非法类型保持严格校验
- runtime-core新增单actor的current准入，验证消息/生成参数、绑定实际ID并立即复用原调度；不在HTTP查询快照后另行提交
- current路径不调用外部文件准备或resolver，不加载/切换/重载模型
- 请求进入事件/worker/IPC前仍有具体ModelId，响应metadata来自实际绑定结果
- 显式ID原本机与LAN行为不变，只有错误提示根据调用入口边界保持准确
- 不修改推理核心、模型ID生成、模型显示名、凭据或LAN可访问范围；没有增加复制ID按钮

## 本地验证

- 作者与独立审查分别执行 `cargo test --locked -p runtime-core -p runtime-api --all-targets`：同一集合133通过、0失败、1既有忽略，不累加
- 新增10项用例，覆盖当前模型准入、DTO空值/非法值、生命周期/控制交错、队列/重复/取消/关停、外部准备隔离及真实TCP接口。既有idle卸载测试追加当前请求拒绝与其他显式ID冲突断言
- 冻结8文件diff SHA256：`8b3e127bfcbd9018d5f07d5bff24604798b651ed9a01a2579d1070922766fae0`，独立审查前后相同；无剩余阻断
- 最终联合 `cargo test --workspace --all-targets --locked`：40组482通过、0失败、7既有忽略；同源码workspace/all-targets严格clippy和fmt通过，均退出0
- 日志：`/workspace/shared/current-model-core-api-tests.log`、`/workspace/shared/current-model-independent-tests.log`、`/workspace/shared/current-model-independent-clippy.log`、`/tmp/nexa-current-model-full.log`
- 同工作树此前UI508项、严格Python190通过/4平台skip及LAN枚举交叉检查通过；本片未改这些UI/Python/bridge文件，不把未重跑阶段说成重新执行

测试使用合成推理执行器，LAN私网端点为测试模拟，HTTP部分确实运行本机TCP。尚未用本片重新运行真实GGUF或原生Windows；不继承旧包实测。

应覆盖：缺省/空/空白/null/非法显式ID；Ready/Generating与空闲/故障/卸载/加载状态；队列及控制命令顺序；独占登记与停服；本机首次显式加载、同ID重载、其他ID冲突；本机/模拟LAN路由真实TCP SSE全部块与非流式实际model字段；external current不进行文件准备。

## 新版交付后的调用检查

1. 显式在Nexa加载模型，记下模型信息中的API ID
2. 向原有Base URL的 `/chat/completions` 发送文本请求，分别省略model、设为空串、设为空白；非流式返回model应等于实际API ID
3. stream=true时检查role、正文、finish及可选usage块中model同样为实际ID，不出现空值
4. 显式卸载后再次发送空ID请求，应明确提示未加载模型，不自动重载
5. 指定另一个ID应保持冲突/非法规则，不静默使用现有模型

以上为待交付测试步骤。用户要求暂缓构建，本轮尚无新的Windows包；旧e0ff1e6仍要求显式model，不能用旧包验证新增行为。
