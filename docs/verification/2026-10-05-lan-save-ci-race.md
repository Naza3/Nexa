# LAN 保存与轮询竞争验证

- 任务：W-CI-LAN-SAVE-20261005
- 基线：`8ff644796df684d4e9bf81f2ae44e56a38ae9552`
- 状态：源码修正、完整前端与重复回归已完成；独立审查通过，无阻断；新 Windows CI 待主代理执行
- 原始失败：[Actions 37288485584 / job 111694576654](https://github.com/Naza3/Nexa/actions/runs/37288485584/job/111694576654) 的 `Check locked desktop frontend`，695 项中 694 通过，`LanApiSettings.test.tsx` 的保存去重测试未找到保存后的启用状态

## 两个独立问题

1. 测试夹具不一致：`setup()` 的默认保存会更新 `snapshot()` 读取的 `current`；该测试改用延迟 Promise 后，返回启用状态却一直保留关闭的 `current`。完成后显式调用轮询共用的 `controller.refresh()`，可确定性重现状态回退。不能以延长等待或重试解决。
2. 控制器存在独立时序窗口：保存期间发起的旧快照读取，与保存 ACK 先后在同一轮微任务完成时，保存任务先发布新值，旧读取可能接着覆盖；外层 `action()` 的 `finally` 此时尚未增加 `snapshotEpoch`。同时 `resolve(save)`、再 `resolve(oldRead)` 的确定性测试得到旧值。ACK 后排队的旧读取失败也必须失效，不能将成功保存后的连接改成错误。

原始 Windows 日志没有完整轮询时序，不能据此把两种竞争中的某一个认定为当时唯一执行路径。两个问题均已分别以真实 `DesktopController` 的 Promise/refresh 路径重现，无延时、测试重试或放宽断言。Rust 桥 `snapshot_gate` 与 `save_lan` 的 `work` 锁独立；保存发布后返回快照的契约不允许测试夹具永远返回旧持久状态，也不赋予前端在途响应新的时序身份。

## 最小改动

- `apps/desktop/src/controller.ts`：仅在 `saveLanSettings` 成功 ACK 后、发布保存快照前同步增加 `snapshotEpoch`，保留原来的 action 起止保护和轮询行为
- `apps/desktop/tests/LanApiSettings.test.tsx`：提供显式夹具状态更新，在 ACK 前模拟持久化；保留重复点击仅一次、完成前关闭和开关禁用断言，增加保存中 refresh 和保存后连续两次 refresh
- `apps/desktop/tests/lanApi.test.ts`：新增旧读取成功/失败 × ACK 前、ACK 同轮微任务、action 完成后三种顺序，共六项；验证去重、保存前状态、保存后状态，并验证随后真实外部关闭设置仍被新 refresh 接受。ACK 前已经观察到的读取错误仍作为有效诊断处理

未改变 LAN 默认关闭、显式启动、凭据、监听、防火墙、原生接口或依赖锁。未改安装器、workflow 或入口状态文档。

## 实际验证

执行环境：Linux 云端，Node `24.19.0`、npm `11.9.0`；工作目录 `apps/desktop/`。日志保存在本次工作区的 `nexa-ci-frontend-review/`，不将构建产物与长日志纳入源码。

| 命令 | 结果 / 退出码 | 证据 |
| --- | --- | --- |
| `npm ci --cache /workspace/shared/nexa-development-tools/npm-cache --no-audit --no-fund` | 239 包，0；package/lock 无变化 | 执行输出 |
| `npm test -- tests/LanApiSettings.test.tsx tests/lanApi.test.ts -t 'deduplicates save and does not replace\|pre-acknowledgement read'`（修正前红测） | 两项均失败，1 | `deterministic-red.log` |
| `npm test`（最终实现） | 33 文件，701 通过，0 | `full-frontend.log` |
| `npm run typecheck` | 0 | `typecheck.log` |
| `npm run lint` | 0 | `lint.log` |
| `npm run build` | typecheck + Vite 构建，0 | `build.log` |
| `npm test -- tests/LanApiSettings.test.tsx tests/lanApi.test.ts`，独立进程连续 10 轮 | 每轮 60 通过，全部 0 | `focused-repeated.log` |
| `git diff --check`，并检查 package/lock 未变 | 0 | 执行输出 |

701 项包含新增六项；focused 检查为其子集，不另计总数。红测只加入强制 refresh 与最小交错反例，尚未加入六顺序矩阵，故过滤结果为两项而非最终七项相关测试。

独立审查核对四文件差异及红/绿日志；另在隔离目录将基线与修正后的真实控制器转译，以 Node 实际微任务执行八组对照，重现旧 ACK 同批成功/失败响应的覆盖，并确认修正和之后的新外部读取生效。审查无阻断；独立对照不另计前端用例总数。

## 验证边界与下一步

- 本记录是前端逻辑/DOM 模拟测试与生产静态构建结果，不是新原生 Windows 安装器或目标 Windows 10 GUI 验收
- 未运行浏览器 localhost，也未执行新 Windows CI、提交、推送或发布
- 由主代理统一提交，在相同锁定依赖下重新运行标准 Windows CI，并继续原安装器验证门禁；不得用本地通过替代 CI 结果
