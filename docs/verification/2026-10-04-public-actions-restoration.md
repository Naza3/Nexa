# 2026-10-04 公开仓库恢复Windows Actions

用户已将Naza3/Nexa改为公开仓库，并明确要求后续恢复GitHub Actions构建。GitHub API已核实 `private=false`、`visibility=public`。这覆盖旧的停用Actions Rust要求，不授权收费larger runner、购买配额或修改预算。使用标准ubuntu-24.04及windows-2022两个顺序job；费用边界见[官方说明](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)。

## 修改

- 现有native-windows只从长期 `codex/nexa-add-model` push或显式workflow_dispatch触发；public-only条件阻止私有仓库误耗额度，同分支concurrency取消旧run
- 固定checkout/upload/download/setup-node action引用保留，checkout实际github.sha、不保留Git凭据，权限仍contents:read；无新增secret、自动merge或Release发布
- Ubuntu真实构建同提交aria2，GITHUB_SHA必须与干净Git HEAD吻合；源码包包括实际两份相关workflow。Windows继续核验同源组件、策略、许可及源码闭包
- CMake使用>=4.2而非精确4.4.3；新CI入口复用既有packager的VS/工具集选择及身份键控native目录，不自行安装VS，不混另一实例工具/CRT。原生测试和后续打包复用真实同目录；本次Python安装的CMake目录通过NEXA_CMAKE_BIN保留，每次VsDevCmd后重新置前并核验cmake.exe/ctest.exe，防止VS自带旧CMake遮蔽新安装工具
- Rust仍固定1.98.1并核对实际release/Windows MSVC host；Node/npm、vendor与依赖锁未变，未夹带pnpm迁移
- 保留所有既有Rust、前端、原生、真实模型、HTTP/CLI、外部模型及提取包验收；修正PDB/原生诊断路径，产物保留7天。Windows job上限120分钟，只提高时间上限，没有增加并行matrix或更大机器
- 三个以原始字节hash锁定的aria补丁使用-text，避免Windows换行转换破坏源身份；未改原patch字节/hash。Python夹具显式UTF-8，不降低EncodingWarning错误门槛

## 推送前实际验证

严格 `python3 -X warn_default_encoding -W error::EncodingWarning -m unittest discover -s scripts -p 'test_*.py'`：170项，168通过、2Windows平台skip，exit0。新CI准备/身份/环境allowlist、同SHA和dirty拒绝、workflow触发/runner/保存路径有回归；py_compile、YAML语义与diff检查通过。

尚未执行新CI；这些静态与Linux合成检查不表示VS、Windows真实模型或完整ZIP已经通过。推送后应记录实际run ID和精确head SHA，只消费对应artifact，失败需保留日志并定位后再试，不能通过跳过检查跑绿。

## 之前的交叉构建状态

2f7478f Linux交叉四EXE和同源aria2已构建、离线源/PE/许可复核通过，但完整ZIP未产生：微软CRT在线CRL访问被策略阻断，正式提权在命令前挂载失败，用户再授权重试仍失败。未禁用校验、未改清单冒充完成。现在按用户新的构建环境要求在GitHub Actions原生构建，不复用旧交叉结果冒充Windows验证。

Windows标准runner测试仍不等于用户Windows10/i5-8400/16GB真实窗口、完全离线、无工具机器或长期稳定性验收；这些保留独立状态。

## 首轮真实运行失败记录

[run37197414719](https://github.com/Naza3/Nexa/actions/runs/37197414719)，head `cbba057b0106b7cc65131332858c0cedc993ff93`，Ubuntu job111422032577成功；Windows native job111422576447失败于组件策略探针，之后Rust/CMake/Nexa/完整包步骤未运行。不是额度未分配机器，也不是之前Linux云端CRT吊销阻断。

实际报告32case、25通过、7失败。原始失败证据artifact11301104741保留于该run。68 policy、26 Request加4 socket、53 payload、公开HTTPS577字节、错误主机名/自签/过期证书三类Schannel拒绝、canonical/mapped私网拒绝及loopback accepted=0均已观察；整体passed仍false。

- integer/hex/octal三个私有地址在Windows getaddrinfo阶段失败，早于SocketCore目标策略门槛；不能声称已执行socket gate
- initial_http/port/userinfo/fragment四项原Windows日志只有resume提示；源码分析及同源Linux固定userinfo单例观察指向Request::parseUri拒绝后的无可用URI DEBUG分支，不是旧探针所期待的ProtocolDetector消息。Windows DEBUG链仍待下一run证明

后续修复仅限探针判定和诊断：精确区分解析器、resolver提前拒绝和实际socket policy拒绝；通用DNS/超时/非零退出或证书错误不能泛化作成功。保持零字节、实际无连接、公开正例和三类TLS负例门槛，新增错误输入/错误host/假日志反例。Linux观察/分类回归不自动改写这次Windows失败，须新run实际验收。

探针修复推送前验证：严格全Python188项，186通过、2Windows平台skip；Ubuntu通配test_aria2_build*.py聚焦37项通过（原19加18），含完整32case合成编排与公网/三证书/listener任一失败总报告必失败。py_compile/diff检查和独立37项子集通过。只修改probe与新增回归，不改生产补丁/来源锁/TLS；旧失败报告不变，回放时旧4个URI日志仍不满足新明确证据要求。
