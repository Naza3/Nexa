# T07-B1 可复现 CI

任务状态：已实现、待 GitHub CI 验证。`.github/workflows/android-mnn-native.yml`
使用独立 Ubuntu 24.04 / GCC 13.3.0 候选配置；允许该配置不等于已通过 CI。
固定 CMake 4.4.3、Ninja 1.13.2、Rust 1.98.1 及 Android Rust target。
显式 `inputs` 阶段复用 T07-A 的 size/SHA 锁定 NDK r30 与五文件模型下载，
随后获取独立 mobile Cargo.lock 的依赖。无新增 SDK 安装或协议接受；已有 NDK
许可审查是前提。此后 Cargo 命令均为 locked/offline。

作业上限 45 分钟，编译最多 2 jobs。每条命令另设超时；超时终止整个子进程组，回收最多等待5秒。清理未确认时标记
`cleanup_unconfirmed` 并阻止后续阶段，不宣称已结束。私有命令日志使用16MiB软阈值，
每50ms轮询且进程快速退出后再次检查；这不是硬字节上限。确认写入停止后截断超限部分，
任何一次读取最多256KiB。
以下阶段及完整证据均为强制门禁：

1. 实际工具/编译器版本、clean commit/tree 身份、helper 测试
2. 显式固定输入与锁定 Cargo 依赖获取
3. pristine 上游到独立私有 patched 副本，精确 postimage 复核
4. 独立未补丁 T07-A Linux 探针与 CTest，并复核原始源码仍 clean
5. Linux C ABI CTest（ABI/sampler、stream、logging macro）、实际编译/头依赖
   日志审计、真实 archive 导出、仅 stream 的 ASan/UBSan
6. 原生真实请求、全部数字取消检查点、与 pristine 探针精确对照、8 项负例
   privacy canary 及成功中英/多轮/取消/callback/恢复 canary
7. Rust fmt、clippy、unit、compile-fail、artifact 负例、固定真实模型测试，
   实际 C 头布局编译并运行，运行 build_identity 示例逐字段对照实际 manifest
8. Android native 交叉构建、日志审计、真实 shim/MNN 与固定
   libc++/c++abi/unwind/Clang builtins 静态闭包导出
9. Android clippy、Rust 最终测试 ELF 完整链接、artifact 负例、C 头布局交叉编译
10. Cargo 实际报告的三个最终 ELF（两个测试及 build_identity 示例）：AArch64 PIE/linker64、依赖仅 libc/libdl/libm、
    全部 LOAD≥16KiB 且地址/偏移同余、无 WX 段/可执行栈、非空 GNU_RELRO
    结束地址按 16KiB 对齐

Rust Android 链接同时设置 max-page-size/common-page-size=16384。
仅 LOAD 对齐不足以证明 RELRO 边界。使用 Cargo JSON 的 compiler-artifact
executable 事件定位三个最终 ELF（两个测试及 build_identity 示例），不猜测 hash 文件名、不以 cargo check 替代链接。
不执行 Android 测试，不声称设备运行。

## 证据及失败保留

`b1_ci.py` 将命令输出捕获到已忽略的 `build/t07b1` 私有日志。
运行 config、合成 prompt/回答/token vector、原始对照报告也仅保留在那里。
native export 生成实际本地 artifact manifest/archive，交给 Rust 严格消费。

上传仅包含重新构造、严格 schema 校验的 JSON：固定检查名、布尔值、计数、数字
采样延迟、源码/patch/policy/header/model/NDK/archive/编译器身份摘要及已验证
ELF 属性。完整实际编译器字符串保留在本地 artifact manifest；上传编译器 SHA256
与固定工具配置。另保留实际 artifact.json 完整字节 SHA256，绑定完整构建闭包。
编译/链接失败额外保留最多16行、合计4096字节的错误诊断，去除绝对源码/工作区/home路径；
仅编译命令可生成此字段，不接受真实模型、panic或聊天输出。其余不上传原始日志、正文、模型、SDK、静态库或可执行文件；原生二进制分发仍等待
license/notice 闭包完成。

staging 在失败后仍运行，保留command index/category/timeout/timed_out/log_limit/cleanup状态、退出码与固定失败标签；
对于已知privacy case、upstream比较字段、candidate/source/export错误仅记录固定标识，删除未经验证的部分嵌套
报告，记录 missing/invalid 报告。只有所有必需步骤/报告成功、源码 clean 且身份
不变时才允许总体成功。完整性门禁失败后仍上传可用安全证据。外部取消可能直接终止
runner，不能保证这种情况下仍有机会完成清理/上传。取消延迟仅为 Linux
checkpoint 到安全返回的样本，不是任意时刻或 Android 真机的取消上限。

## 触发隔离

Windows `paths-ignore` 仅排除明确孤立路径：
`native/mnn-{probe,shim,patches}/**`、`scripts/android_mnn/**`、`mobile/**`、
两个 Android workflow、`docs/**` 及明确列出的根文档。
GitHub 仅当**全部**改动匹配排除列表时跳过；混合 Android/docs 与 Windows 代码
的提交仍运行 Windows。`packaging/**`、`crates/**`、`native/llama-shim/**`、
根 Cargo/CMake 输入、Windows workflow 以及未知新路径均仍触发。
本次 legacy UTF-8 修正涉及 `native/llama-shim/**`，因此 B1 提交仍运行 Windows。
没有修改 Windows permissions、timeout、job 或验证门禁。

## 实际本地验证

2026-10-02 执行以下命令，均 exit 0：

```sh
python -m unittest discover -s scripts/android_mnn -p 'test_*.py'
actionlint .github/workflows/android-mnn-native.yml .github/workflows/native-windows.yml
```

Python 共 59 项（1 项已有环境条件 skip）；包括缺失/失败/非法报告、嵌套原文泄露、
重复 outcomes、dirty source、遗漏取消阶段、非有限时间、错误 ELF 架构/依赖、
不足 LOAD/RELRO 对齐及可执行栈，另覆盖快速日志超限、读取上限、超时回收、kill/退出竞态、
无法确认回收后的阻断，以及编译错误脱敏/真实模型输出禁止进入诊断。另断言 Windows 精确排除列表、全部已知 Windows
输入与未知路径仍触发。现有真实 native 日志的 10 个取消样本也经新 parser 复核。
此处未重新编译 native 或执行 GitHub CI；下一步由主代理审查后运行完整工作流。
