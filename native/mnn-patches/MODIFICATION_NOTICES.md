# 上游修改文件显著标记与环境恢复

2026-10-02。13文件修改标记源码已精确恢复；本节所述历史通过与恢复后的重新验证必须分开。

## 历史修改和不变约束

在原13个patch目标第一行加入`Nexa modifications (2026-10-02)`及各自用途。去掉新增首行，每份字节与旧postimage完全相同，所有上游版权保留。

- 旧Nexa基线：`fc8d87291404ea9b97cb5c5d18b35c0596ab8bc9`
- 旧patch-set：`43cc33146e2036ff452bd02d5ec352bb099d143ed4a4cdeb6ff55335987f9ce0`
- 新/恢复后的patch-set：`dfe571d08b1583e39d7ce271eb289ebc91c06b88261a3c83fdef1e53dc062b80`
- 三patch SHA分别为`1710b0483a5e8a6ec9b0aa8fa0fecab28ddec68d3d0737f391f26b7ae93686d5`、`90e19a7350abed88bf82482b83d52f570c582dd2b60c289dc170115f0639025d`、`6216f089d7af26053a5c062243735625ca914dbbf0e256e864eb76db991816b6`，恢复时逐字节hash全部吻合
- 不变：MNN commit `d407447ed56c4121a11ccbd266dc184ca1ead0c2`、13份before SHA256、ABI1、policy正文/hash和C ABI header字节
- policy SHA：`ea06621b78e67e58f97f98951b26db0a8a893ded4112da3e5a762b98566fa328`
- header SHA：`40d1a79df99f5200d92e689baf791d5105d2da71d521fcd885a2807f0b916b8e`

[源码等价记录](modification-notice-verification.json)保存全部旧/新postimage及去注释hash。恢复后的13文件精确重放、私有source postimage、原版clean检查、fc8历史lock/header不变量交叉核对均已重新执行通过。

```sh
python -B native/mnn-patches/verify_modification_notices.py --source /path/to/clean-MNN-3.6.1
python -B native/mnn-patches/identity.py --source /path/to/private-patched-MNN --verify-post
```

验证器仅临时复制13个文件，先检查before，再精确`git apply --check`和`git apply --whitespace=error`，核对postimage，并剥离新增注释比较旧postimage；不修改原版checkout。

## 环境损失与历史证据边界

06:34 UTC执行环境整体替换为空，原repo/shared构建产物、模型、工具链均消失。源码随后从GitHub精确恢复至`f4fa90abad4e1e58ea785508251026083ae62177`，本13文件增量依据此前工具文本和再次取得的原版源码重建，三patch hash及patch-set完全复现。

此前两平台旧B2 artifact确曾完整复制到`/workspace/shared/nexa-mnn-b2-preserved-artifacts/{linux,android}`并逐文件校验（Linux3文件、Android8文件）。**这些保留副本也已丢失，当前不存在**。不得重新生成“已copy保留成功”的报告，或把重建出的文件冒充旧副本。

原始`modification-notice-build-verification.json`和原始日志同时丢失。现同名JSON是明确标注的历史摘要重构，不是原报告逐字节恢复；缺失的完整copy文件表、原日志hash和archive字节没有伪造。保留的历史要点：Linux CTest3/3、真实中英/多轮/预算/采样/取消恢复、重新运行原版逐token对照、9项privacy、stream ASan/UBSan曾通过；Linux314/242、Android445/220日志审计，Android r30/API28/arm64构建、export/hash及16KiB LOAD曾通过。历史notice库存12组件/27文件通过。

历史新身份产物hash仅供对照，不代表当前文件存在：

| 项目 | Linux | Android |
|---|---|---|
| shim | `43261b20f10692789196ed5024fb287d7a0ff7f8e05832289fa88bcebeb0c081` | `44b21c0f83e69562fe10588519eb8697e215102eb204e2b770ebec110aaa5d3c` |
| MNN | `75e6328cbfa5ebaa9f83224d6b030b19247e7e8cfce86e78dd56d4eb2ad801cd` | `22a11893a1b64ca125dacaa388f2c774b947e810d23573c4639d135669df9f2e` |
| manifest | `b8b4d8efb06388f49c6457d177997f2bf630c5dceeb9ec190d3cc245a313372d` | `6ff7a9625cd8bf1135e3f82fa36095da4f4f27ebc0ac808c474020e62e1cb750` |

## 恢复后验证

当前：源码/hash/notice等价恢复已通过。Android已重新完成r30/API28/arm64构建、445/220日志审计、archive hash/export与全LOAD0x4000检查；其manifest和6个archive实际重现历史dfe字节身份，记录在[恢复后新证据](recovery-build-verification.json)。这是新环境实编，不是从旧文件copy恢复。Linux也已重新完成构建、CTest3/3、真实请求、9项privacy、stream ASan/UBSan、314/242审计、export/hash，manifest及两份archive实际重现历史dfe字节身份。独立未打补丁probe也已从干净原版重新构建并实际运行，完整模板、21个prompt token IDs、12个output token IDs、文本与usage全部匹配。至2026-10-02 07:20 UTC，本轮全部原生恢复门禁已重新通过，原版MNN保持clean；旧43cc B2保留副本仍不存在。

旧B2测试完整BuildIdentity门禁未改。新native源码或原生验证都不自动授予新Rust/B2研究身份；Rust/产品/手机/App验收属于独立后续工作。此次不修改App/mobile/CI，不提交或推送。


### 恢复后的实际命令与证据

两平台仍使用原路径`/workspace/shared/nexa-mnn-shim-{linux,android}`和同一CPU profile，以`cmake --build ... -j2`从新环境重编。新原始证据统一在`/workspace/shared/nexa-mnn-recovery-*`，每份输出SHA及实际退出0记录在`recovery-build-verification.json`；不采用丢失日志的旧hash冒充新执行。

- Linux：`ctest --test-dir ... --output-on-failure`为3/3；`mnn-request-test CONFIG REPORT`真实矩阵通过；`privacy_canaries.py`为9项；`-fsanitize=address,undefined`的stream测试通过（`detect_leaks=0`，不声称全MNN/LSan）
- 原版baseline：`native/mnn-probe`对干净`d407447...`单独构建，真实运行后`compare_upstream.py`通过；没有用同一patched库冒充独立原版
- 日志：Linux314单元/242头、Android445/220审计通过；Android仅静态审计，未运行设备logcat
- Android：NDKr30/API28/arm64交叉构建；`llvm-readelf -l -d`全部LOAD为0x4000；固定资源目录`lib/clang/21/lib/linux/aarch64/libunwind.a`正确；6个archive和manifest逐hash验证
- 导出：均在build/audit通过后执行。Linux manifest实际SHA为`b8b4d8efb06388f49c6457d177997f2bf630c5dceeb9ec190d3cc245a313372d`，Android为`6ff7a9625cd8bf1135e3f82fa36095da4f4f27ebc0ac808c474020e62e1cb750`，重新生成的库和manifest确实重现历史dfe字节身份。该结果不恢复旧43cc B2 backup

新Rust/B2 receipt、App链接、产品准入和手机执行不是本次原生恢复任务的通过声明；由对应独立任务提供证据。本任务无stage/commit/push。
