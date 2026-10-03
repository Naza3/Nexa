# Windows 范围收敛前的历史索引

日期：2026-10-03。此目录保留收敛前的混合平台设计、当时进度及证据链接；不再驱动当前 Windows 开发。当前规则见 [ADR0014](../../decisions/0014-windows-desktop-cpu-runtime.md) 与 [当前状态](../../../PROJECT_STATE.md)。

快照原始字节的 SHA-256 如下。归档文本仅增加历史说明和修正相对链接；源码、既有验证报告和 Android 未提交工作未搬迁、未删除、未修改。

| 原路径 | 原始 SHA-256 |
| --- | --- |
| [AGENTS.md](AGENTS.md) | `f10c80016f3eb942cdba2e25655d6be03da9cbc703d7330afbd1f7cce549ae99` |
| [README.md](README.md) | `c5137e67dcc38af0817f9b4d55502280ae5cd86984854c9cc060fdfd56bd7ca3` |
| [PROJECT_INDEX.md](PROJECT_INDEX.md) | `ca15cba3b6b10a2de3fbcd6457820d68e3fe0fb66bc9ce13f50b4566e6473129` |
| [PROJECT_STATE.md](PROJECT_STATE.md) | `981cf1dbfedeab68add851ed66057757fa7943c566da7003b0cf666e03013558` |
| [docs/architecture.md](docs/architecture.md) | `f33464a760dd438e2190355b8397c66e23c77f8eb4a0605ad2aee6f0c09f86d3` |
| [ai-runtime-v0.1-execution-spec.md](ai-runtime-v0.1-execution-spec.md) | `59fd05c71111967d463639fa1ca5834fc92dd7af0cd6bf81a8c00e9bbaf60333` |
| [docs/roadmap.md](docs/roadmap.md) | `0420d01a23a1ea2a15fc2acd795177ad261c569fdcb443c675aceb5c27d0162d` |
| [docs/build-lock.md](docs/build-lock.md) | `7a1fa1fb7c5608438ab365687654ece41f9f9fbd2d01364ef5e347cac89befca` |
| [docs/model-matrix.md](docs/model-matrix.md) | `e7a6c44c444c90ddad659b0974fc72321fb4f1c98b6721c742607411a3e43d4a` |
| [docs/telegram-summary.md](docs/telegram-summary.md) | `de2f1eeb6af5af1e8b3a48b77eb65c9d682073e47ec2f7ded04bc05a83defceb` |

## 保留在原位置的历史研究

以下链接用于历史追溯，均不构成 Windows 发行前置，也不授权恢复开发：

- [Android MNN 方向 ADR0008](../../decisions/0008-android-mnn-engine-and-package.md)
- [Android 产品 ADR0009](../../decisions/0009-android-mnn-chat-product.md) / [原产品计划](../../android-app-parity.md)
- [Android MNN 计划](../../t07-android-mnn-plan.md) / [运行契约](../../t07b-mnn-contract.md) / [设备验证计划](../../t07c-android-verifier-plan.md)
- [T07-A 报告](../../verification/2026-10-02-t07a-mnn-cpu-probe.md) / [T07-B 报告](../../verification/2026-10-02-t07b-mnn-runtime.md) / [设备交付报告](../../verification/2026-10-02-android-device-verifier-delivery.md)
- [暂停方向 ADR0013](../../decisions/0013-windows-focus-and-android-mnn-chat.md)

`apps/android-verifier/` B3b 未提交 WIP、`mobile/runtime/`、MNN 原生目录和隔离 CI 原位保留，不作为 Windows 源码清理对象。外部 MNN Chat fork 是独立项目。
