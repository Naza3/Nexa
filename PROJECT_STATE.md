# Nexa 当前状态

最后更新：2026-09-30。状态区分工程实现、Linux 开发验证、Windows CI 与目标设备验收，不把任何一项互相替代。

## 当前目标与授权

用户已明确要求按照文档规划实施，并允许本地检查通过后推送新的开发分支、增加和运行 Windows GitHub Actions。未授权合并或部署。首个业务仍是 Telegram 群摘要；来源、触发、样本与保留策略保持待决，不自行登录账号或向群发送摘要。

目标设备保持 Windows i5-8400 / 16GB，以及用户描述的 Android 骁龙 8E5 / 12GB；准确系统、手机型号、ABI/页大小和持续性能未实测。

## 已有工程事实

- 原始远程提交 `0d3a3cea32b813dad0857f9e1a1e41862ce27168` 已通过 GitHub 原始对象精确重建本地 Git；初始 tree/commit SHA 一致，未创建替代历史
- 最小 Rust workspace 已落地：runtime-types、llama-adapter、xtask；无 runtime-core、worker、HTTP、UI 或移动工程
- llama.cpp submodule 锁定 `2149c00f4442dc59302e134a02e4c99d5f7ed9fc`；Rust 1.98.1 与 Cargo.lock 已锁定
- 自有 C ABI 实现模板、分词、精确逻辑预算、prefill/decode、采样、跨 token UTF-8/stop、取消和资源释放；Rust 使用借用/线程约束及 panic 隔离
- Qwen3-0.6B Q8_0 实际文件和模板 hash 已核对；来源、许可与固定参数见 [模型矩阵](docs/model-matrix.md)
- Linux CPU 已完成真实上游中文非思考生成及 native suite；没有 Windows/Android 真机结果
- 已准备 `.github/workflows/native-windows.yml`；对应授权分支为 `codex/nexa-native-baseline`。工作流尚需父任务提交、推送并检查实际 CI 结果，不能以文件存在宣称通过

构建参数见 [构建锁](docs/build-lock.md)；当前可执行命令见 [xtask](xtask/README.md)；验证见 [本轮记录](docs/verification/2026-09-30-native-baseline.md)。

## 任务状态

| 任务 | 状态 | 当前边界 |
| --- | --- | --- |
| D00 文档与总体设计 | 已完成 | 原文档基线保持；当前工程事实已同步 |
| T00 工程与基线 | 待验证 | workspace/锁/真实模型/Linux上游已落地；Windows上游基线与构建组合待CI实测 |
| T01 原生链路 | 待验证 | 原生和Rust封装、真实中英文/长输入/取消/stop/恢复均有Linux证据；T00目标平台门槛尚未补齐，不算完整阶段完成 |
| T02–T04 调度、存储、worker、API | 未开始 | 待前置门槛；未预建空模块冒充实现 |
| T05–T06 Windows发行与UI | 未开始 | 仍需PC全链路及独立验收机 |
| T07–T08 Android核心与UI | 未开始 | 无Android工具链/真机，本轮build.rs明确拒绝Android目标 |
| T09 发布验收 | 未开始 | A01–A26完整矩阵未执行 |
| T10 平台/后端扩展 | 未开始 | 本轮Linux仅开发探针，不是扩展平台发布 |
| S00–S04 摘要 | 未开始 | 来源/触发/样本/质量目标及接口契约仍待冻结 |

## 重要实现与验证界限

- 模型的原生 context 分配会向上按256取整；Nexa 单独保存用户请求的逻辑context预算，真实33-token边界回归已覆盖，不能借分配扩容放宽预算
- 上游 `llama-completion --reasoning off` 未在初始prompt分支传关闭参数；基线使用锁定原模板渲染的固定合成prompt。自有shim直接关闭思考，不剥离输出标签
- 目前回调为同步借用；T02/T03的异步队列背压、deadline、慢消费者时限、崩溃隔离均未实现
- 生成中跨线程取消已有功能测试；独立prefill中途取消延迟、100短请求/20加载的长期内存趋势、5次性能统计及目标硬件表现未完成
- ASan/UBSan纯流缓冲测试通过；LeakSanitizer因沙箱ptrace不可用，未宣称原生库通过完整内存泄漏检测
- 摘要文本质量、证据归因、Android后台生命周期、正式Windows发行能力均未验证

## 下一步

1. 父任务复核本轮最终验证、中文提交，推送已授权的新开发分支并跟踪Windows Actions
2. 若Windows编译或真实suite失败，修复同一范围并重验，不跳过检查；WindowsCI通过也不替代i5-8400和独立无开发工具验收机
3. 补T00/T01目标平台证据后按路线进入T02；Android交叉编译探针需实际工具链，不编造APK或真机结果
4. 摘要来源/触发/评估基线保持独立待决；基础runtime推进不依赖自行选择Telegram产品方案
