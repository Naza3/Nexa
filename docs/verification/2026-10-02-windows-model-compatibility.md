# Windows模型兼容性说明与API示例修正

日期：2026-10-02。状态：本地逻辑/前端回归通过；最终提交Windows构建与原生UI验收另行记录。

## 范围

Windows继续使用原有本地HTTP服务、llama.cpp/GGUF与最小管理UI。本片仅涉及桌面，不新增推理后端或放宽当时的精确模型准入。

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


## 最终提交、CI与交付

实现提交 `389eeef327f00a184bb644ceacbfaa310f39f780`，tree `1935158e31c16da7d9b9a44afd2ac02c21809ff8`。30个Windows代码/规范/路线文件。

[Windows37009292638](https://github.com/Naza3/Nexa/actions/runs/37009292638)于13:29:41 UTC成功（attempt1）。证据artifact11229030777：77,737 bytes，SHA256 `e928a13293ce3c3d1a6c5245aee7763ac80ded1d1823ab5ace674f1b675e2a96`。root独立核对50项库存大小/hash、封闭文件集、精确source与全部成功结果；真实模型/取消恢复/store/worker/HTTP-CLI、Release包和仓库外提取产品/desktop bridge成功，desktop package_unchanged=true、native_window_tested=false。两个原有optional/失败路径报告缺失边界保持。

桌面artifact11228098955：8,774,798 bytes，SHA256 `31ad23895e20eb416ae093651c47c6233737822facd3b711e710d0bff91e68cc`。其中实际产品ZIP为9,789,508 bytes，SHA256 `45251f28c2eb61a1b6ee5119aab3b0923a8117c677fef4ec91ea680be1b209f0`，750个文件。root逐文件核对manifest/SHA256SUMS、嵌套runtime同source/完整性、6个x64 PE，以及540个桌面Rust、6个npm、186个runtime许可记录摘要。Linux复核时只将原验证函数的路径排序适配为PureWindowsPath顺序，未修改产物或放宽库存/hash检查。

以 `Nexa-Windows-x64-389eeef.zip` 原字节交付，2026-10-02 13:36 UTC附件发送被接受；这不等于用户已下载或运行。提示退出旧UI及服务后解压到新文件夹。包不包含模型，Windows10新界面/目录/剪贴板手验仍待完成，既有后期条件不扩张。
