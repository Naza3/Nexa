# 2026-10-09 LAN 启动降级恢复验证

任务：W05-LAN-RECOVERY-1。范围与公共行为见 [ADR0042](../decisions/0042-lan-bind-startup-recovery.md)。本记录仅为 LAN 切片的主机验证，整体错误处理联合门槛须另行记录。

## 实现与已验证边界

- LAN OS bind 按四类有界错误报告，保持认证回环服务；配置错误与回环绑定失败仍失败关闭。
- 注入 binder 单测核对只尝试原配置地址一次、保留本机 socket 可连接、禁用 LAN 不尝试第二次监听、成功显式尝试无旧错误。
- 实际 CLI 子进程在不可用私有 IP 下运行：发现仅回环，实例锁有效，认证 status 与 stop 成功；配置字节、本机 token 和已有 LAN token 不变；首次失败不创建 LAN token；显式停止释放锁/发现，再禁用 LAN 启动无旧诊断。不包含实际模型推理。
- bridge DTO 兼容旧字段缺失，未知诊断码降为有界通用失败。UI 保留模型就绪与本机连接，实际 LAN 未运行；取消停服不动作、确认仅一次、停止失败不解锁配置、停止成功不自动重启或保存。

## 本轮命令

- `cargo test --locked --offline -p runtime-cli -p desktop-bridge`：退出0，192通过/1项真实 OCR 测试按原条件忽略，不能当作实测；数量包含执行时已落盘的其他 bridge 错误改进测试。
- `npm test -- --run tests/lanStartupRecovery.test.tsx tests/lanApi.test.ts tests/lanAddresses.test.ts tests/runtimeProjection.test.ts`：退出0，4文件108项通过，含7项新恢复测试。
- `npm run typecheck`：退出0。
- `npm run lint`：LAN 切片初次退出0；最后联合工作树检查遇另一个正在实现的 `errorPresentation.ts` 控制字符正则 lint 错误，已交该文件负责人修复，不能据初次结果宣称最终全树通过。
- `git diff --check`：退出0（LAN代码冻结时）。

原始主机日志存于本次执行环境 `nexa-tooling/provenance/lan-startup-recovery/`。测试后新增的其他错误处理改动须参与最终联合测试。

## 待验证与交付边界

未推送、未创建 tag/Release、未更改版本。全量主机/前端/Python及两 workspace Windows strict Clippy、原生库与四 EXE 实际交叉链接，待其他切片冻结后统一执行；之后仍须独立原生 Windows CI 与用户目标 Windows 10 GUI/真实局域网条件验证。这里的协议/合成测试不等于远端可达或真实模型验证。

## 联合验证收尾

随后所有切片冻结后，[整体错误处理联合验证](2026-10-09-error-handling.md)已通过根Rust652/10忽略、壳33、前端1006、严格Python366项（361通过/5skip）及完整Windows交叉门槛。上述切片的中途lint错误已修复并在全树lint复验通过；切片计数不再叠加。真实成功LAN重绑定仅有注入binder测试，实际CLI测试证明的是失效LAN保留本机及显式关闭LAN后重启清错；未声称真实网卡恢复实测。
