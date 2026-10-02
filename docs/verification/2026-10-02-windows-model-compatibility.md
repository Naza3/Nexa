# Windows模型兼容性说明与API示例修正

日期：2026-10-02。状态：本地逻辑/前端回归通过；最终提交Windows构建与原生UI验收另行记录。

## 范围

Windows继续使用原有本地HTTP服务、llama.cpp/GGUF与最小管理UI。按照ADR0013暂停独立Android App产品开发；本片不混入其未提交WIP，不新增推理后端或放宽精确模型准入。

ModelSummary新增受控compatibility值：admitted、architecture_unsupported、quantization_unvalidated、template_unvalidated、context_unvalidated、artifact_unvalidated、unvalidated。原因由受控登记manifest纯计算，不在列表阶段增加整文件读取/hash；未准入availability_error仍用既有unsupported_model。实际文件/目录失效原因优先，prepare失败刷新不洗掉更具体原因；load/chat继续拒绝未准入模型。

bridge对旧服务缺字段及未来未知枚举保守转为Unknown。UI移除“尚未校验”歧义，分别说明登记元数据、引擎范围与精确矩阵准入；已登记hash不被说成当前文件完整性证明。当前准入仍仅固定Qwen3-0.6B Q8_0，Qwen3.5等另行验证。

手动API示例原先两个终端各生成新GUID数据目录，第二终端无法读取第一实例令牌；现由终端A显示目录，B读取同一路径，不输出令牌，不初始化用户长期服务。

## 验证

- 选定四crate共144项Rust测试通过，0失败：desktop-bridge56、model-store37、runtime-api43、runtime-types8
- 同范围clippy严格告警检查通过，format与diff检查通过；Cargo.lock未改
- HTTP契约验证import/list兼容性一致、load/chat继续拒绝候选；external消失/替换优先原错误
- 前端固定Node24.19.0/npm11.9.0，npm ci/lint、74项测试、typecheck/build均通过；最后布局换行样式后已重新执行全套
- 独立审查无阻断；独立复跑22个不同Linux逻辑/契约测试通过，涵盖manifest、external、prepare、HTTP与bridge兼容解析
- 云浏览器访问本机Vite被ERR_BLOCKED_BY_CLIENT阻止，未完成像素级预览，不绕过；测试Vite已关闭。组件测试不代替原生Windows窗口

本地检查未运行本片真实GGUF推理或Windows原生UI。需要精确提交的Windows真实模型、打包、解压bridge与产物独立验收；Windows10目录选择、自动名、零复制、剪贴板仍不能由旧版手验追溯证明。无开发工具、实际离线、长期稳定性和Windows11仍保留后期范围。
