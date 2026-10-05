# ADR0014：Windows 桌面 CPU runtime 与 API 接入主线

日期：2026-10-03。状态：按用户最新方向采纳，用户已授权“可以，现在逐步推进”；后续功能尚未因此实现或通过验收。本次W00仍为纯文档，实施切片独立记录。

## 背景

用户要求回到 Nexa Windows 版，专注桌面处理器，以 llama.cpp 为核心，移除 Android 相关设计并规划后续内容；并明确要求 API 兼容 deepseek harness。现有主文档仍把跨端共享、Android 真机及完整聊天产品作为发行依赖，与当前目标冲突。

## 决策

1. Nexa 当前产品是 Windows 本地 LLM runtime，为其他应用提供本机 API。桌面 UI 从已有验证壳逐步完善为模型/服务管理器，聊天是辅助验证入口。
2. Windows 10 x64 / i5-8400 是首要目标；后续扩大 Intel/AMD 桌面 CPU 实测矩阵与 Windows 11 验收。处理器性能、指令集及推荐参数必须实测，不把任意 x64、Intel 或 AMD 标为已支持。
3. 固定 llama.cpp 为实际推理核心，沿用自有 C++ shim 和已有 Rust 服务层。Rust 管理模型、安全、单 actor 调度、队列、取消/超时、worker 生命周期、HTTP/CLI 与桌面桥；不重做 tokenizer、采样或计算内核，不为移动复用继续扩大抽象。
4. 保留现有 Executor/ModelResolver 等有实际 Windows 用途的边界；不以“去 Android”为理由删除稳定调度层、改成第二个调度器或无证据重写为 llama-server 转发。
5. 原文档收敛阶段仅移出跨端发布依赖；2026-10-05按用户新要求由[ADR0029](0029-desktop-only-source-tree.md)进一步移除本项目移动源码、专用CI与文档，替代原地保留要求。独立 `Naza3/MNN` fork 与Git历史不动。
6. 当前不排 GPU/NPU、移动端、其他操作系统、模型市场、账户/云同步或完整聊天产品。Telegram 摘要作为可选参考调用端，不阻塞 runtime 发行。
7. 目标已核实为官方deepseek-ai/deepseek-harness（dsh），以dsh-v0.2.0-rc.2精确commit为研究基线。优先使用dsh-llm-pi-ai的自定义openai-completions provider，不先新增默认deepseek-official所需Messages网关；版本/配置/字段与H01–H12见[harness契约](../windows-harness-contract.md)。现有文本子集不足以完成工具闭环。
8. W02明确包含桌面实用模型准入，当前0.6B只证明链路；4B等候选先核固定llama架构/模板/量化、内存/质量并单独验工具能力。新路线使用 W00–W05，不把已完成 T00–T05、已实现 T06 能力重新包装成待开发。新包原生 UI 待验范围保留；无开发工具、离线和长期稳定性按用户要求放到后期。

## 影响与验收

- 替代原跨端范围中与当前桌面产品冲突的未来排期；保留桌面历史验证事实，不将旧成功结论转授新源码
- 同步入口、架构、执行规格、路线、构建锁和模型矩阵；旧版可由Git追溯，不在当前树维护移动归档
- Windows 协议、C ABI、模型准入、鉴权与进程安全行为此次不变。现有 API 尚未宣布 harness 兼容；新增行为必须另有契约/实现/回归证据
- 文档任务仅做一致性、路径/链接和变更检查，不运行大型构建、不制造新二进制，不将文档通过写成功能通过
