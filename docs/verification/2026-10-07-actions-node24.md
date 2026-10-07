# GitHub Actions 原生 Node.js 24 升级

任务 W05-CI-NODE24-1。用户要求消除旧 Action 声明 Node.js 20、被 runner 强制以 Node.js 24 运行的弃用警告。实施基线为 `codex/dev` 的 `c99e98678c7e8b9914636784e155d9fc3bc03e49`，已包含最新 main `02c90da687d8b4d493b8d071f46bc8710597055f`。产品版本保持 `0.2.1`。

## 修改与官方来源

四个自有工作流共 27 处调用全部按完整提交 SHA 固定。已通过官方 release/tag API 核对版本与提交，并读取各精确提交的 `action.yml`，四项均声明 `runs.using: node24`。

| Action | 官方版本 | 固定提交 | 调用数 |
| --- | --- | --- | --- |
| checkout | [v7.0.1](https://github.com/actions/checkout/releases/tag/v7.0.1) | `3d3c42e5aac5ba805825da76410c181273ba90b1` | 8 |
| setup-node | [v7.0.0](https://github.com/actions/setup-node/releases/tag/v7.0.0) | `820762786026740c76f36085b0efc47a31fe5020` | 1 |
| upload-artifact | [v7.0.2](https://github.com/actions/upload-artifact/releases/tag/v7.0.2) | `cf430e030ddbb5b0abf93d22962f4752f3646cd9` | 15 |
| download-artifact | [v8.0.2](https://github.com/actions/download-artifact/releases/tag/v8.0.2) | `9000827ccba6bdab643e8b6fd33ac0654aef8333` | 3 |

范围为 `.github/workflows/native-windows.yml`、`aria2-build-probe.yml`、`aria2-windows-probe.yml`、`modelscope-route-probe.yml`。

- 所有上传显式 `archive: true`，保持具名、多文件 ZIP artifact；下载仍按原精确名称解压到原路径。
- 下载显式 `digest-mismatch: error`，保留 v8 的严格默认值；摘要不符直接失败，不降回旧版警告行为。
- setup-node 显式 `package-manager-cache: false`，避免新版 npm 自动缓存。产品前端 Node `24.19.0`、npm `11.9.0` 不变；它们与 Action 自身的执行 Node 是不同配置。
- 保留 checkout 的 `persist-credentials: false`。checkout 新版对 `pull_request_target` / `workflow_run` 的 fork 限制不涉及本项目现有触发。
- 触发、runner、权限、条件、产物名称、路径、保留期、版本/tag 门禁均不变。Node24 Action 要求 runner 至少 `2.327.1`；现有标准 GitHub 托管 Ubuntu/Windows runner 符合官方支持范围。

## 本地验证

以下检查退出码均为 0：

1. `/tmp/nexa-actionlint/actionlint -color .github/workflows/*.yml`：四个工作流语法与 Action 输入检查通过。临时工具为官方 actionlint `v1.7.12`，下载归档 SHA-256 已核对为 `8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8`。
2. Python/PyYAML 一次性结构审计：将新配置中的 27 个 SHA 和 19 个显式输入归一化后，与 `git show HEAD:<workflow>` 完全相等；另核对上述官方 `action.yml` 的 Node24 声明与新增输入存在。
3. `python3 -B -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'`：297 项，292 通过、5 个既有平台跳过，8.192 秒。
4. 独立只读审查：归一化逐字比较通过，无遗漏或阻断；`git diff --check` 通过。

## 验证边界与下一步

工作流及本地检查已完成，精确提交的 GitHub 托管原生执行待验证。本地检查不代表新版本的实际上传/下载、Windows 完整构建或 tag 发布已经通过；三个可选探针工作流未额外触发。

按既有授权推送 `codex/dev` 后由现有分支触发 CI；用户合并到 main 后，后续从该代码创建的 tag 才会使用新 Action。旧 run 的警告和旧 tag 引用的工作流不会随分支更新消失。
