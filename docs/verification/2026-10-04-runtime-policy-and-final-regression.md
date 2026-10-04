# 2026-10-04 运行策略与模型管理联合回归

基线为长期开发分支 `codex/nexa-add-model` 的 `abcb1a0a9b448915cf311e4cf33c427cf6af017b`。本批按ADR0023新增文件校验超时与显式空闲卸载开关，保留此前LAN、所选文件添加和本机测试证明修复。

## 实施与审查

配置默认校验300秒（30..7200秒），idle启用且300秒。缺字段默认兼容；旧手工较长idle读取不收紧，新桌面保存仍1..86400秒。两项单独停服持锁重读保存，不覆盖LAN或对方字段。

扫描/添加/下载完成后登记/外部模型加载前准备使用冻结的整操作deadline；取消/身份/原子发布保持。idle=false仅跳过TTL卸载，显式卸载/切换/停服仍可释放。不以0表示disabled。

本机已证明连接，仅external登记模型四个精确POST路由可将响应头等待设为校验预算+750秒，最高7950秒。未知方法/路由/预算拒绝；managed/状态/纯短测和SSE读等待不扩大。服务端body/native/生成计时及LAN loaded-only准入未改变。

独立审查发现旧长TTL读取收紧和重复点击成功仍残留busy提示，均在冻结前修复并回归。壳命令注册/build/ACL/Python集合一致，新增且仅新增 `runtime_verification_save`，共34项。

## 实际验证

- 父全workspace/all-targets：40组464通过、0失败、7既有忽略
- 父全workspace/all-targets clippy `-D warnings`、fmt、diff检查exit0
- 前端完整typecheck/lint、20文件414项测试、production build exit0（343基线加71）
- Python155项：153通过、2Windows平台skip
- 作者六crate338/0/1、壳宿主31/0/0，两处clippy/fmt通过
- Windows六crateall-targets与Tauri壳all-targets交叉check exit0；壳有既有clang-cl编译器family识别警告，记录为警告，不等同实机通过
- 独立复用binary验证Rust42/42、前端83/83，均为上面全量子集，不累加
- 固定Linux原生库与全新release runtime/worker/desktop-harness构建exit0

新版Linux真实GGUF harness exit0，生产Python报告消费者也通过：success/real_model/local_text_validation/repeat_text_validation/offline_inventory全部true；首次输出139字节、usage54 tokens，取消保留6字节、重复输出75字节，关闭/保留独立runtime/显式回收均通过，stderr为空。输入固定Qwen3-0.6B Q8_0，639446688字节，SHA256 `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`，2048context/2线程/128batch。此为Linux managed模型真实链路，external_library.supported=false；Windows外部load/test/锁与离线证明未因此通过。不是旧1845报告，也不推广至任意GGUF。代码冻结后构建，提交前工作树测试与最终Windows交付来源分别核验。

## 未验证和交付规则

Windows实际执行、原生窗口/文件选择/共享锁、两机LAN、7200秒真实慢盘、无开发工具/完全离线/长期稳定性和Windows11均未验。本机合成测试不替代这些条件。目标机清单见 `2026-10-04-windows-model-management-checklist.md`。

最终clean提交后重新捕获来源，真实重建同提交aria2，再以现有锁定Clang/MSVC ABI环境构建Windows四EXE、核验PE/CRT原件签名/许可/生产消费者并打包。未使用GitHub Actions Rust，不强推main。源码验证不等于二进制已经构建或发行验收完成。
