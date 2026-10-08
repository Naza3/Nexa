# 顺序图片OCR验证（2026-10-08）

任务W02-OCR-6。从 `2e6a69b` 开始，在codex/dev实现桌面多图队列；用户最终要求默认按导入顺序，不按文件名排序。设计见[ADR0037](../decisions/0037-sequential-image-ocr-queue.md)，操作见[OCR说明](../ocr-windows-cpu.md)。版本仍0.2.3；交付前fetch origin/main=`1c3650c`，开发分支已包含main（本轮开始ahead9/behind0），无需合并冲突或重写历史。

## 范围与验证层级

仅修改桌面前端、controller、本轮测试与说明。复用现有单图OCR请求、指标和100条历史存储，无Rust/HTTP/IPC/shim/ACL/依赖变化，不重复跑无关原生构建或把前轮真实模型结果当成本轮验收。

Linux云环境复用已安装Node24.19.0/npm11.9.0及 `/workspace/onboarding/nexa-env.sh`。测试使用有控制时序的模拟桌面API；浏览器实际使用Chromium、File输入和PNG读取/预处理，生成回复与磁盘API为明确模拟。Windows WebView2文件窗口的返回顺序、用户Win10/i5-8400耗时和真实多页识别质量仍待目标环境验证。

## 实际检查

| 命令 / 场景 | 结果 | 证据 |
| --- | --- | --- |
| `npm test` | 退出0，45文件958项通过 | `/tmp/nexa-batch-frontend-full.log` |
| 最后 `npm test -- --reporter=dot tests/OcrBatch.test.tsx` | 新增2项边界后19项通过；属于定向子集，不与全量加总 | `/tmp/nexa-ocr-batch-tests.log`、`/tmp/nexa-batch-final-regression.log` |
| `npx vitest run tests/ocrReservation.test.ts` | 4项通过，退出0 | `/tmp/nexa-ocr-batch-reservation.log` |
| 既有OCR/历史/摘要三个文件定向 | 74项通过，退出0 | `/tmp/nexa-batch-existing-final.log` |
| `npm run typecheck` / `npm run lint` / `npm run build` | 均退出0，最后改动后复核 | `/tmp/nexa-batch-typecheck.log`、`/tmp/nexa-batch-lint.log`、`/tmp/nexa-batch-build.log` |

批量19项覆盖：FileList顺序10/2/1、手动上下移/移除、同名文件独立身份、数量和单图限额及20×4MiB边界、逐图准备、非终态不能推进、正文保存完成才下一张、迟到指标与查看结果归属、失败/停止后只继续未开始项、断流/坏终态保留原request并恢复、准备期间停止/关闭阻止迟到提交、切页继续、关闭取消当前并保存部分正文、落盘失败暂停、逐图检查外部换模以及256KiB溢出保留部分正文和最终明确警告。

controller4项覆盖本窗口图间互斥、错误/旧token不得释放新批次、并行管理或外部生成时不得取得占位、状态读取/关闭收尾不自锁，以及旧poll先结束再读取新快照以避免把上一张generating状态错当下一张状态。既有单图重跑保持，重跑使用新条目ID阻止上一轮迟到指标覆盖。

## 浏览器与审查

`python3 /tmp/nexa-ocr-batch-browser.py` 退出0。真实Chromium检查：导入10/2/1保留原序，调序恢复、start/save严格交替、跨页运行与返回、每图正文/指标独立、复制只有原文；第一图失败暂停后显式继续只提交剩余两图，不重放失败项。1440/1024/390宽度无横向溢出，队列滚动限定在360px，pageerror为空。多图已导入时明确显示待逐张准备，不再误报尚未选图。

证据：`/tmp/nexa-ocr-batch-browser.py`、`/tmp/nexa-ocr-batch-browser.json`、`/tmp/nexa-ocr-batch-browser.log`、`/tmp/nexa-ocr-batch-{1440,1024,390}.png`。主代理人工查看宽/窄截图。本轮新启动Vite因1420已有该仓库的预览进程而退出1，随后复用已存在的明确预览服务；没有停止不属于本轮启动的进程。浏览器fixture只在工具临时文件/页面注入，不进入产品源码。

独立审查关闭了单图重跑复用itemId造成旧指标串入新结果的问题；逐图fresh snapshot和停止/关闭检查收口。全量中必要调整仅把旧同步调用断言改为等待实际提交，并将正文保存前不可开启新任务落实到旧历史测试，保留原有请求次数/所有权断言。构建保留既有单JS产物超过500KB提示。

最终 `git diff --check` 与修改文档240个本地链接/代码围栏检查通过。本轮不自动推送、创建tag或发布Windows包。
