# W05-CI-NET-1：公网 TLS 检查有限重试

日期：2026-10-07。本地源码及 Windows 组件/脚本检查已通过，完整应用构建进行中。用户要求减少外部证书测试偶发失败；开发基线已同步 main `02c90da687d8b4d493b8d071f46bc8710597055f`，产品版本仍为 `0.2.1`。

## 实际故障与范围

- [Actions37605103921](https://github.com/Naza3/Nexa/actions/runs/37605103921) 首次 Windows 尝试32项中31项通过，`self-signed.badssl.com` 返回 `exit=2` / `Timeout` / 零字节；同提交第二次尝试整体通过
- [v0.2.1 Actions37610183607](https://github.com/Naza3/Nexa/actions/runs/37610183607) 首次32项中30项通过：错误域名样例发生连接重置 `(2746)`，过期证书样例超时，均未取得预期证书错误。它与上述成功源码只差文档，tag 不改变该早期检查

以上证据能定位网络层观测，不能确定故障来自站点、runner 还是中间路径。原分类器拒绝把普通连接失败当成证书拒绝是正确行为。本次只改 `third_party/aria2/tests/windows_probe.py` 的重试控制与诊断，未修改生产 aria2、三份补丁、来源锁、证书/吊销参数或应用版本。

## 已实现行为

只有固定公网正例 `example.com` 及三个 badssl 证书负例可以重试。最多三次，第二/第三次前等待2秒/5秒，每次使用独立空目录；保留单次 `--max-tries=1`、15/20秒网络超时和60秒子进程超时，不自动重跑整个工作流。

授予重试资格需零响应字节、匹配退出码、唯一当前 URL 的 `[ERROR]` abort 行、紧邻且唯一的原生错误链。只接受实际 `AbstractCommand errorCode=2 Timeout` 或明确的 Winsock 2745/2746/274c 连接中断/超时。额外错误、错 URL、证书/吊销错误、未知 HRESULT、DNS/HTTP/策略错误、已有响应字节和子进程超时均立即失败；不等待后续尝试覆盖它们。

每次仍调用原 `classify_download`。证书负例最终必须取得绑定当前URL、正确原生来源及该样例指定 Schannel 证书错误的证据；网络错误永不算通过。三次仍为可重试故障时以 `network_retries_exhausted` 失败，进程超时为 `process_timeout`。四个公网检查均继续作为整体32项门禁的一部分。

报告外层保留原最终结果字段，追加 `attempt_count`、`recovered_after_retry` 与 `attempts`；每次保留编号、退出码、字节数、判定、网络故障类别及有界诊断。即使恢复通过也保留先前失败；已有脱敏证据上传能递归保留这些字段。日志只输出固定样例名、次数、原因和摘要，便于直接定位网络重试。没有新增环境依赖或用户配置。

## 实际开发验证

工作目录 `/workspace/Nexa`，Linux 云环境：

| 检查 | 结果 |
| --- | --- |
| `python3 -B -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'` | 297 项，292通过/5平台skip，退出0；日志 `/tmp/nexa-tls-retry-full-tests.log` |
| 聚焦 `test_aria2_build_policy_windows.py` | 26项通过，退出0，包含原18项与8组新重试回归；属于完整suite子集 |
| 聚焦 `test_stage_ci_evidence.py` | 23项通过，退出0；真实重试报告经既有stage脱敏后仍保留失败历史与恢复证据 |
| 两份实际 Actions 报告中的三条失败诊断重放 | 均准确识别可重试网络故障，原 `classify_download` 仍为失败，退出0 |
| 独立只读审查 | 31项日志/歧义反例及5种控制场景通过；为分层证据，不累加到297项 |

实际报告下载位于 `/tmp/nexa-37605103921-evidence/windows-aria2-policy.json` 与 `/tmp/nexa-37610183607-evidence/windows-aria2-policy.json`，不把临时文件路径作为提交测试的依赖。永久回归使用合成日志和模拟原生进程，但执行真实分类器、重试控制、目录隔离、参数构造和报告清洗。

已验证恢复后立即停止、三次耗尽、每次空目录、TLS/进程参数保持、真实错误证书立即失败、响应字节不重试、进程超时保留历史并停止、错误链混杂/隔断/倒序不重试。独立审查发现的“额外无errorCode错误仍可能重试”已收紧并补反例；没有删断言或弱化通过条件。测试捕获模拟失败输出，避免把合成诊断误当成 CI 自身失败。

## 验证边界与交付

离线模拟和旧 Windows 日志重放与新源码在 Windows 执行分别记录。精确提交 `c8163f452192c06e1271945c43577562b3c1d21c` 已推送，实际 [Actions37614473400](https://github.com/Naza3/Nexa/actions/runs/37614473400) 的 `Verify Windows component policy and prepare product closure` 和 `Install locked Rust and compatible CMake tools` 均成功；分别执行新探针及完整严格 Python suite，非零退出即抛错。记录时完整应用/安装器构建仍进行中，最终报告尚未读取，不能由步骤成功推断本次公网检查是否实际发生过重试。

本次重试控制的成功恢复及拒绝错误由上述离线/Windows测试分别覆盖；完整产品结果另据 Actions。持续外网故障仍可能阻断检查，此改动减少偶发故障，不保证第三方站点可用。随后仅补记验证结果的文档提交不冒称经过上述精确源码原生运行。

现有 `v0.2.1` 指向旧提交，重跑该 tag 不会获得新增重试逻辑。本轮不移动 tag、不合并 main、不发布 Release；后续包含本次修复的提交/版本自动使用该逻辑。
