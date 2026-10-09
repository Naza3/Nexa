# 模型名称与 PI Desktop 选择器

任务W05-MODEL-NAME-1，2026-10-09。本轮仅修正Nexa模型发现接口缺失名称；不修改独立OCR插件或PI Desktop客户端。

## Nexa接口

普通文件添加/扫描已有可读`display_name`，默认取GGUF文件名；内部稳定ID通常为`ext-UUID`。配对导入GLM-OCR时用户填写的`glm-ocr-q8`是可读ID，并非该模型另有专属名称协议。

本轮本机和LAN的`GET /v1/models`每条记录增加已有`display_name`，保留`id/object/owned_by`。本机分页、generation与可用模型范围保持；LAN仍只返回当前驻留模型。名称只用于展示，同名记录按不同ID分别保留，调用、配置、历史和加载均继续使用原ID。不存在按名称模糊匹配或名称白名单。

名称复用model-store已验证的非空、至多1024 UTF-8字节显示名，不解析新GGUF字段、不迁移索引、不覆盖自定义名称。LAN状态有ID但注册表名称暂不可用时回退到原ID，不返回空名称。响应仍不展开manifest、源路径或额外元数据。

## PI Desktop两处界面不同

已直接核对官方v0.17.0及当前主分支`ed54ce9774c41cefb5d0cdcdf922de7e6df22621`；这不是用户安装版本或真实桌面运行的证明。

- **设置中添加模型**：`normalizeModelList`读取`display_name`，缺失时回退ID；发现列表展示不同于ID的名称，并支持按名称搜索。因此Nexa补字段能改善此处。使用包含本修复的新服务后，客户端需要重新获取模型列表，旧缓存不会自行被改写。
- **聊天模型选择器**：PI Desktop明确使用已配置的`alias`，未配置时显示完整ID。它不以服务返回的显示名替代ID；本次Nexa改动不能单独让这里自动显示名称。当前主分支仍是该规则。

现有客户端配置方法：打开设置中的模型服务配置，编辑Nexa服务，在已选模型行点击**高级**，填写**别名**，点击**保存服务**。别名仅改变显示，实际请求ID保持。中文界面这些按钮/字段名已由v0.17.0翻译与组件代码核对；不同安装版本的布局可能不同。

上游证据：

- [发现响应解析](https://github.com/vastsa/PI-Desktop/blob/v0.17.0/apps/desktop/electron/main/model-discovery.ts#L129-L145)
- [发现列表名称与搜索、已选模型的高级/别名](https://github.com/vastsa/PI-Desktop/blob/v0.17.0/apps/desktop/src/components/settings/ModelSelectionPanes.tsx)
- [保存模型绑定](https://github.com/vastsa/PI-Desktop/blob/v0.17.0/apps/desktop/src/components/settings/ProviderSetupDialog.tsx#L288-L329)
- [聊天名称选择规则](https://github.com/vastsa/PI-Desktop/blob/v0.17.0/apps/desktop/src/lib/composer-models.ts#L29-L78)
- [别名归一化与持久模型绑定](https://github.com/vastsa/PI-Desktop/blob/v0.17.0/crates/host-core/src/providers/catalog.rs#L32-L86)、[保存到config_json](https://github.com/vastsa/PI-Desktop/blob/v0.17.0/crates/host-core/src/providers/credentials.rs#L228-L247)

本轮验证层级见[验证记录](verification/2026-10-09-model-display-names.md)。GLM-OCR实际识别仍使用本机`POST /v1/chat/completions`；OCR适配器为加载、取消和性能等附加能力使用`/runtime/*`，与本次主客户端名称问题分开。
