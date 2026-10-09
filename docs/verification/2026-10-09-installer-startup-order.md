# 2026-10-09 安装验收启动顺序与诊断

任务W05-ERROR-CI-1，基线`085cf4024ec5f742e8b46e91210dd3511de01f8c`。

## 原生失败证据

[CI 37926350189](https://github.com/Naza3/Nexa/actions/runs/37926350189)四项前置任务通过，native在NSIS重复安装后的runtime_ready失败。MSI相关阶段、NSIS安装/重复安装和已安装EXE诊断此前通过，但完整生命周期未完成，不能宣称安装包已验证。原stdout/stderr均丢弃，无具体启动原因，因此不把后续复现倒推为此次失败的确定根因。

独立下载核对runtime/desktop证据闭合清单、hash/大小和源码身份。Windows根Rust655通过/10忽略、桌面Rust34、严格Python366项（364通过/2skip）、CTest5项；各子集不相加重复计数。

## 复现与修复

受控持有同一instance.lock时serve立即exit1，固定错误为another instance owns this data directory；释放锁后等待runtime/instance.json再认证status成功，stop成功、serve wait0且discovery删除。CLI的离线status会短暂取得该锁，原验收立即status可能抢在serve前取得锁。既有Rust生命周期测试已先等待Discovery。

- 验收只接受全新data目录，init后确认无旧discovery。
- 保留30秒deadline及child存活检查；新discovery只允许开始探测，仍由原status完成认证/实例/endpoint证明。
- 不改端口、不重试serve、不绕过锁、不跳过安装验收。
- 私有启动协议增加固定runtime_instance_busy；桥和UI提供占用提示。stdout仅在已退出时读取最多257bytes，未知/额外/超长内容统一generic，不公开原文；保留stderr null。
- 失败记录规范化32位退出码和受控错误码，临时输出关闭；原stop/wait检查保留。

## 验证状态

18项脚本定向和4项实际CLI启动测试通过。完整宿主回归：根Rust654通过/10忽略、桌面33通过、前端49文件1006通过、严格Python370项（365通过/5平台skip）；两个workspace fmt/strict Clippy和前端typecheck/lint/build均退出0。测试子集不重复加总。保留既有大chunk warning。

两workspace Windows严格检查/四EXE实际链接将绑定干净提交后执行，通过前不更新远端分支；后续原生CI仍待本轮结果，不以旧CI代替。
