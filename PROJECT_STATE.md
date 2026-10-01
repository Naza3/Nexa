# Nexa 当前状态

最后更新：2026-10-01。状态区分工程实现、Linux 开发验证、Windows CI 与目标设备验收，不把任何一项互相替代。

## 当前目标与授权

用户已明确要求按照文档规划实施，并允许本地检查通过后推送新的开发分支、增加和运行 Windows GitHub Actions。未授权合并或部署。首个业务仍是 Telegram 群摘要；来源、触发、样本与保留策略保持待决，不自行登录账号或向群发送摘要。

目标设备保持 Windows i5-8400 / 16GB，以及用户描述的 Android 骁龙 8E5 / 12GB；准确系统、手机型号、ABI/页大小和持续性能未实测。

## 已有工程事实

- 原始远程提交 `0d3a3cea32b813dad0857f9e1a1e41862ce27168` 已通过 GitHub 原始对象精确重建本地 Git；初始 tree/commit SHA 一致，未创建替代历史
- Rust workspace 已有 runtime-types、model-store、runtime-core、engine-host、llama-adapter、xtask；无独立 worker、HTTP、UI 或移动工程
- llama.cpp submodule 锁定 `2149c00f4442dc59302e134a02e4c99d5f7ed9fc`；Rust 1.98.1 与 Cargo.lock 已锁定
- 自有 C ABI 实现模板、分词、精确逻辑预算、prefill/decode、采样、跨 token UTF-8/stop、取消和资源释放；Rust 使用借用/线程约束及 panic 隔离
- Qwen3-0.6B Q8_0 实际文件和模板 hash 已核对；来源、许可与固定参数见 [模型矩阵](docs/model-matrix.md)
- Linux CPU 与 Windows x64 CPU CI 已完成真实中英文流式、重复加载、stop/预算/取消/恢复和 A02 system/多轮模板与请求间隔离
- 提交 `d3d7cf2d9f0d2ce7aa03ca7d27787a2f423f3144` 的 [Windows CI](https://github.com/Naza3/Nexa/actions/runs/36791679663) 全部通过：固定模型/模板身份、上游生成及五次 bench、自有九场景/十轮 suite、两项真实恢复/多轮测试
- 本次支持证据限 Windows Server 2022 x64 / EPYC CI / 2 逻辑 CPU、2 推理线程、固定 Qwen3-0.6B Q8_0 / context 2048。1 线程短探针通过，4 线程超配短探针仍在 60 秒超时，不能泛化为任意线程配置；原始失败完整保留于报告

构建参数见 [构建锁](docs/build-lock.md)；当前可执行命令见 [xtask](xtask/README.md)；验证见 [本轮记录](docs/verification/2026-09-30-native-baseline.md)。

- T02 已实现受控模型导入/manifest、单一调度actor、有界FIFO/输出、三类deadline、空闲卸载与专用原生线程；Linux真实store→core→host→shim链路通过，Windows新增验证尚待本次提交CI
- shim build_info升2，保留旧air_generate并新增数值进度观察；Linux在成功prefill批次后与decode阶段分别跨线程取消，实测见 [T02记录](docs/verification/2026-10-01-t02-runtime.md)

## 任务状态

| 任务 | 状态 | 当前边界 |
| --- | --- | --- |
| D00 文档与总体设计 | 已完成 | 原文档基线保持；当前工程事实已同步 |
| T00 工程与基线 | 已完成 | 固定组合已在 Windows CPU 完成真实上游生成和五次 bench；输入、统计与构建证据已归档 |
| T01 原生链路 | 已完成 | 最小阶段门槛通过 Windows 真实中英文流式、重复加载释放、模板/特殊 token、取消和恢复；A08独立prefill已补Linux观测，随T02复验Windows |
| T02 调度与存储 | 待验证 | 实现及Linux逻辑/真实GGUF链路已通过；等待本次Windows CI与最终整合复验 |
| T03–T04 worker与API | 未开始 | 待T02门槛后实施独立worker与HTTP，当前线程执行器不冒充进程隔离 |
| T05–T06 Windows发行与UI | 未开始 | 仍需PC全链路及独立验收机 |
| T07–T08 Android核心与UI | 未开始 | 无Android工具链/真机，本轮build.rs明确拒绝Android目标 |
| T09 发布验收 | 未开始 | A01–A26完整矩阵未执行 |
| T10 平台/后端扩展 | 未开始 | 本轮Linux仅开发探针，不是扩展平台发布 |
| S00–S04 摘要 | 未开始 | 来源/触发/样本/质量目标及接口契约仍待冻结 |

## 重要实现与验证界限

- 模型的原生 context 分配会向上按256取整；Nexa 单独保存用户请求的逻辑context预算，真实33-token边界回归已覆盖，不能借分配扩容放宽预算
- 上游 `llama-completion --reasoning off` 未在初始prompt分支传关闭参数；基线使用锁定原模板渲染的固定合成prompt。自有shim直接关闭思考，不剥离输出标签
- 回调仍为同步借用，现允许共享预算内可取消等待；T02实现256KiB预算、4KiB UTF-8分片、10秒无消费进展时限及协作式deadline；Windows五秒kill/进程崩溃隔离仍属于T03
- Linux已补独立prefill中途取消与decode取消观测；100短请求/20加载的长期内存趋势及目标硬件表现未完成；Linux数据仅代表共享开发机
- ASan/UBSan纯流缓冲测试通过；LeakSanitizer因沙箱ptrace不可用，未宣称原生库通过完整内存泄漏检测
- 摘要文本质量、证据归因、Android后台生命周期、正式Windows发行能力均未验证

## 下一步

1. 主代理整合T02并执行完整workspace检查，按授权推送开发分支并等待新增Windows CI真实调度/存储测试；失败先修复再复验，不把Linux替代目标平台
2. T02门槛通过后推进T03：独立worker、握手、NDJSON、有界管道及五秒取消强杀/崩溃隔离
3. Windows CI仍不替代i5-8400、T05独立无开发工具验收机或Android真机；平台/支持矩阵不能泛化
4. 摘要来源/触发/评估基线保持独立待决；基础runtime推进不自行选择Telegram产品方案
