# Android 产品能力对标与阶段

日期：2026-10-02。方向见[ADR0009](decisions/0009-android-mnn-chat-product.md)；实际进度见[状态](../PROJECT_STATE.md)。这是产品计划，不代表已有APK。

## 参考基线

固定官方MNN commit `d407447ed56c4121a11ccbd266dc184ca1ead0c2`（引擎3.6.1）中的Android MNN Chat版本为0.8.3。本次读取的[官方当前README](https://github.com/alibaba/MNN/tree/master/apps/Android/MnnLlmChat)同样列0.8.3；网页可能缓存，未取得当前master HEAD，未完成全量差异审计。以下“已有”指源码/官方发布记录，不代表本项目在手机上实测。模型目录条目不等于任意设备支持。

## 能力矩阵

| 能力 | 官方固定源码/说明 | Nexa目标 |
| --- | --- | --- |
| 模型目录 | 搜索、标签、模型卡/大小，资产/缓存与联网刷新 | P1小规模已验证目录；P2增强筛选/来源 |
| 下载 | Hugging Face/ModelScope/Modelers、进度/速度、暂停恢复、重试/更新/删除；HTTP Range和持久化 | P1单公开源续传，完整校验后准入；P2多源/更新 |
| 本地与存储 | “添加本地模型”实际是ADB复制说明及目录扫描；模型/配置/mmap清理 | P1原生URI导入私有目录、大小/空间预检、模型与缓存分类 |
| 聊天与历史 | SQLite多会话、历史重开/删除、新建、流式/停止/复制、Markdown/LaTeX、切模型 | P1持久化多会话及文本闭环；P2增强呈现/切模型 |
| 重生成 | 找到字符串，但菜单项被注释，未确认有效实现 | 作为Nexa自身需求设计，不冒称上游已交付 |
| 上下文/设置 | keep-history、system prompt、最大输出、采样/线程/精度/mmap、思考模式 | P1仅已实现参数和精确预算；P2逐项验证高级设置 |
| 后端 | 普通下拉CPU/OpenCL；QNN模型及额外库加载链 | CPU→OpenCL→QNN分档；直接Hexagon单独实验，不声称官方App通用开关 |
| 视觉 | 图片/多图、部分模型视频输入、实时语音视觉 | P3按单图→多图/视频→实时视觉推进 |
| 音频/语音 | 音频输入/部分模型音频输出；ASR/TTS实时语音及语音模型管理 | P3独立ASR/LLM/TTS链与中断/权限验收 |
| 生图/编辑 | diffusion文生图、Sana图像编辑 | P4独立任务类型、资源预算与资产准入 |
| OCR | 未确认独立OCR/扫描入口 | 视觉读字仅按模型实测，不标专用OCR已对齐 |
| API/性能 | OpenAI/Anthropic兼容服务、benchmark、排行榜上传 | 本地诊断先行；网络服务/上传不自动纳入 |

主要固定源码：[模型市场](https://github.com/alibaba/MNN/tree/d407447ed56c4121a11ccbd266dc184ca1ead0c2/apps/Android/MnnLlmChat/app/src/main/java/com/alibaba/mnnllm/android/modelmarket)、[下载器](https://github.com/alibaba/MNN/tree/d407447ed56c4121a11ccbd266dc184ca1ead0c2/apps/frameworks/model_downloader/android/src/main/java/com/alibaba/mls/api/download)、[本地添加](https://github.com/alibaba/MNN/blob/d407447ed56c4121a11ccbd266dc184ca1ead0c2/apps/Android/MnnLlmChat/app/src/main/java/com/alibaba/mnnllm/android/main/MainActivity.kt#L531)、[聊天](https://github.com/alibaba/MNN/tree/d407447ed56c4121a11ccbd266dc184ca1ead0c2/apps/Android/MnnLlmChat/app/src/main/java/com/alibaba/mnnllm/android/chat)、[重生成菜单](https://github.com/alibaba/MNN/blob/d407447ed56c4121a11ccbd266dc184ca1ead0c2/apps/Android/MnnLlmChat/app/src/main/res/menu/chat_context_menu_user.xml)、[设置](https://github.com/alibaba/MNN/tree/d407447ed56c4121a11ccbd266dc184ca1ead0c2/apps/Android/MnnLlmChat/app/src/main/java/com/alibaba/mnnllm/android/modelsettings)。

## 阶段与完成条件

### P0：内部工程基线（T07-A/B）

独立CPU探针→生产C ABI/MnnExecutor/资产schema/模板与精确预算/每请求采样/安全取消。Linux、交叉构建、Android运行分层记录；缺导出器身份、原始模型revision或设备证据时不能授予生产支持。无设备可继续独立设计/实现，但依赖门槛不消失。

### P1：首个日常可用文本APK（T07-C/T08）

- 小规模验证目录、一个公开下载源、进度/暂停/续传/失败重试、URI导入、空间预检、完整包校验、加载/卸载/删除与存储统计
- App本地多会话数据库、新建/重开/删除、完整多轮、流式/停止/复制、清空当前上下文；重生成若纳入本阶段须明确旧答案处理，不自动重放中断任务
- CPU基础参数、上下文/输出预算、耗时/吞吐与可恢复错误；不显示未实现开关
- 下载/导入中断与重启恢复、不完整包隔离；会话不串线、超长明确提示；后台取消/安全卸载、旋转与进程重建、飞行模式真实聊天
- T07-B生产安全门槛及目标设备A19/A21–A23/A26验收完成；能安装的APK或一块生成文本框不足以通过

下载最初仅前台显式操作，不自动增加下载前台服务或后台推理。模型资产准备完成后才能承诺离线；不把外部资源拉取留给模型运行配置。

### P2：文本体验与可选加速

多源/模型更新、筛选、缓存清理、切模型、增强文本呈现、逐模型思考模式和高级参数。OpenCL/QNN按T07-D/E独立推进；直接Hexagon遵循T07-F实验门槛。CPU产品不等待全部后端完成，每个后端具备真实执行/取消/内存/质量记录才公开支持。

### P3/P4：多模态与高级任务

P3依次单图问答、多图/音频、ASR/TTS、视频/实时视觉；P4文生图/图像编辑及可复现本地性能套件。每类扩展版本化输入输出、资产引用闭包、内存预算、权限、停止与生命周期；不是沿用文本接口即可宣称支持。网络API、账号/云同步、遥测/排行榜另行决策。

## 离线与隐私

官方普通本地聊天未见强制账号，但下载器未证明私有/gated模型授权可用。目录刷新、下载、更新与在线模型卡需要网络；视觉/语音离线还要求对应辅助模型和库就绪。官方源码存在按构建开关与用户同意启用的[分析事件](https://github.com/alibaba/MNN/blob/d407447ed56c4121a11ccbd266dc184ca1ead0c2/apps/Android/MnnLlmChat/app/src/main/java/com/alibaba/mnnllm/android/utils/AnalyticsTracker.kt)及排行榜上传，不能把“本地推理”扩大成整款App绝不联网/上传。Nexa不自动继承这些功能，摄像头/录音/媒体等权限按需要请求，不照搬安装更新或广泛存储权限。
