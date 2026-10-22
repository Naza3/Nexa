# ADR0043：模型发现响应携带显示名

- 日期：2026-10-09
- 状态：实现及本地主机/Windows交叉联合验证完成；原生目标设备待验
- 任务：W05-MODEL-NAME-1

用户在PI Desktop添加/选择Nexa模型时只能看到内部ID。Nexa管理摘要已有显示名，但本机及LAN的`/v1/models`只输出`id/object/owned_by`。上游v0.17.0实际发现解析与添加列表支持`display_name`；其聊天选择器另行规定使用用户别名或完整ID。

决定为两处`/v1/models`兼容增加已登记的`display_name`，不改稳定ID、请求匹配、分页、可用性或LAN仅驻留模型范围。名称不唯一，不成为别名；缺少驻留名称元数据时回退ID。复用原model-store名称校验，不新增GGUF读取、迁移或客户端修改。

不采用直接以名称替换ID：会破坏配置/历史兼容，并引入重名、字符和持久身份歧义。也不声称服务端字段足以改变PI Desktop聊天标签；该处可使用客户端已有的高级→别名配置。具体[接口与操作说明](../model-display-names.md)、[执行规格](../../ai-runtime-v0.1-execution-spec.md#7-http-与客户端契约)和[验证记录](../verification/2026-10-09-model-display-names.md)分别维护契约、边界与实际证据。
