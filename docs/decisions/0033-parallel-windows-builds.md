# ADR0033：Windows CI 缓存与并行构建

日期：2026-10-07。状态：已采纳，实际 Windows 结果见[验证记录](../verification/2026-10-07-ci-performance.md)。

用户要求按构建耗时分析依次优化。完整构建继续使用公开仓库的标准 `windows-2022` / `ubuntu-24.04` runner，版本和所有现有测试、真实模型、许可、来源及安装器门禁保留。

## 构建顺序

`release-identity` 成功后，`desktop-build` 与 `download-component` 同时开始；aria2 成功后 `runtime-build` 开始。`native` 等待两个 Windows 构建和 aria2，校验输入后组装桌面包并执行真实 bridge、MSI/Setup 和安装生命周期检查。只有 tag push 可进入原有 `release` job。

桌面 job 运行前端 lint/test、独立 Rust test/clippy 和 Tauri Release；Tauri 的 `beforeBuildCommand` 承担唯一一次前端生产构建。Runtime job 保留全部原生构建、workspace、真实模型下载/推理、HTTP/CLI 及独立解压验收；桌面 Release harness 使用 packager 相同的 Cargo target 目录和显式 MSVC target，复用 Release 依赖。

## 缓存和产物

缓存 npm 下载数据、Cargo registry/git 数据和两套独立 Rust 编译目录。Cargo 凭据、配置、模型、CMake 原生树和最终发行目录不进入缓存。缓存键按实际工具链、runner image、构建路径和锁文件分区；缓存命中后仍执行 Cargo 编译/检查及全部测试，不把命中当成来源证明。

Rust 编译缓存主键追加提交 SHA，让新的 main 构建保存本次更新的编译结果；恢复时优先相同锁文件，再回退同角色/同工具链。锁文件或版本变化时，由 Cargo 按新锁文件和指纹决定哪些依赖重编。`main` push 也运行完整检查并预热默认分支缓存：GitHub 允许 tag 读取默认分支缓存，但不允许新 tag 读取 `codex/dev` 或另一个 tag 的缓存。首次 main 构建仍可能是冷缓存；待 main 对应构建成功保存缓存后再打 tag，发布构建才能复用它。新增 main 触发不授予分支发布权限。规则依据见 [GitHub 缓存访问范围](https://docs.github.com/en/actions/using-workflows/caching-dependencies-to-speed-up-workflows#restrictions-for-accessing-a-cache)。

跨 job 的临时 handoff 只包含桌面 EXE 或两个完整 Runtime ZIP/校验文件及桌面验证器。固定库存记录每个文件的 SHA256/长度，并绑定干净源码提交、工作树字节摘要、同一 Actions run 与实际兼容工具链身份。不同 run、源码、工具链或文件内容拒绝；同 run 的失败 job 重跑可以消费早先成功 job 的产物。ZIP 在临时目录按封闭路径规则展开并重新验证包清单、许可和来源后才落地。

工具链兼容身份与缓存分区分开：前者比较实际 Rust/CMake、MSVC、SDK/UCRT 和 CRT 文件身份；后者额外区分 runner image、VS 安装实例和路径。runner 镜像标签相同不能替代工具链校验。

最终组装 job 重新准备固定工具和 npm/Cargo 许可原文，从同 run 的已校验 aria2 输入准备组件；固定公开测试模型再次下载并核对 SHA256。这不替代 Runtime job 对产品下载路径的真实测试。中间产物保留一天；脱敏证据按 desktop-build、runtime-build、最终 native 分别保存，原始运行目录和模型不上传。

## 权衡

新增 runner 初始化、有限产物传输和最终验证模型下载会产生开销；收益必须以冷缓存和命中缓存的完整实际运行分别衡量，不能将两个并行 job 的用时相加作为用户等待时间。Release 优化参数与产品功能不变。目标 Windows 10 / i5-8400 体验仍需独立验收。
