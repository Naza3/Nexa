# T07-A MNN CPU 探针：开发验证与Android交叉构建

日期：2026-10-02。结论：独立C++ CPU探针已实现；Linux真实模型功能和Android arm64原生构建通过。**T07-A仍进行中，真机运行待验证；预转换资产的导出来源未知单独披露；没有APK或生产MnnExecutor。** 产品目标另见[对标计划](../android-app-parity.md)。

## 实现与身份

探针本体6文件：`native/mnn-probe/{CMakeLists.txt,probe.cpp,README.md}`、`scripts/android_mnn/{candidate-model.json,run_probe.py,test_probe.py}`。复现入口见[README](../../native/mnn-probe/README.md)。CI辅助文件另行记录，不把6视为整轮最终变更数。Windows源码/公共Rust协议未因该探针改变。

- MNN 3.6.1 commit `d407447ed56c4121a11ccbd266dc184ca1ead0c2`，洁净外部checkout、无补丁；构建拒绝错误HEAD/脏源码
- Linux x86_64 / GNU14.2.0；Android NDK r30 `30.0.16248370` / Clang21.0.0 / arm64-v8a / API28；CMake4.4.3、Ninja1.13.2、Release静态MNN/LLM
- CPU、text-only、load-time greedy、precision=high、2线程；HTTP资源、OMNI、视觉/音频及GPU/NPU关闭；上游Linux默认SSE/AVX2不构成任意x86兼容承诺
- 公开预转换Qwen3-0.6B五文件/revision/hash及模板锁见[模型矩阵](../model-matrix.md)；exporter commit与原始Qwen revision未知，按[ADR0010](../decisions/0010-model-artifact-and-conversion-provenance.md)不宣称转换可复现，但不单独阻塞运行资产准入；生产安全/设备/许可门槛仍未完成，候选不是生产准入包
- Linux探针SHA256 `555c9127ceec6932313eefe1b62b6246b59f2f53213e27610e3cfcec55883509`
- Android探针SHA256 `c022f793e4efe84245ef72db117d3269dc5a8c50603f21cba93ada26631f6450`

## 已执行与结果

| 检查/命令 | 结果与级别 |
| --- | --- |
| README所列CMake configure/build，Linux及Android分别构建 | 两目标成功；上游有未使用参数等warning，不称零警告 |
| `python -m unittest discover -s scripts/android_mnn -p 'test_*.py'` | 探针本体19项通过，退出0；主审独立复核通过。后续CI辅助测试数量另计 |
| `ctest --test-dir <Linux构建目录> --output-on-failure` | 2项通过，退出0；主审独立复核通过 |
| `run_probe.py --case short-en/short-zh/multiturn/empty --threads 2 --max-new-tokens 16 --logical-context 512`（分别独立输出目录） | 4项真实MNN加载/模板/tokenizer/生成成功，退出0 |
| short-en相同参数重复 | 退出0，输入token与输出hash和首轮一致；不是跨设备/随机采样保证 |
| short-en、输出预算8、逻辑context36/35 | 28+8=36成功，退出0；35拒绝，退出1，原生错误`logical_context_exceeded`，是预期负例 |
| 主审独立short-zh | 退出0；31输入/7输出token（含终止token），报告身份一致 |
| Android ELF程序头 | AArch64原生CLI，所有LOAD段align=0x4000；仅16KiB链接对齐证据 |
| Android探针CI | 未运行；本地成功不能记作CI通过 |

真实用例观测：short-en为28输入/9输出，short-zh为31/7，multiturn为44/9，empty为12/16；输出计数含MNN终止token，不作为公共usage冻结定义。首轮英文加载约1.774秒、生成约0.304秒只是共享Linux宿主单次功能观测，不能作为手机性能。

本地忽略证据目录包含`t07a-final-{short-en,short-zh,multiturn,empty,repeat}`、`t07a-boundary-{36,35}`与`t07a-parent-zh`的report.json。报告标明android_validated/exporter_commit_verified/thread_safe_cancel/per_request_sampling均false。合成结果和私有native日志不整体发布；日志可能含绝对路径。本文只保留脱敏结果/身份，不将日志原文当产品数据。

## 许可与分发边界

本轮未发布原生二进制、模型或SDK。`MNN_BUILD_OPENCV=OFF`不等于移除MNN内部`source/cv`实现；Unicode衍生表等内嵌数据的来源/许可与最终再分发闭包仍待审计。构建成功、CPU/text-only和上游顶层许可不能代替实际链接内容的许可核验。

## 关键安全范围与未完成门槛

- native用同一owner thread创建/加载/渲染/分词/生成/释放，精确预算消费同一token vector；Python golden仅核验，不替代native模板
- harness验证固定5文件/hash、已知embedding引用范围，重建受控临时配置，不修改源包；拒绝额外文件/context引用和symlink。尚无生产TOCTOU防护/不可变导入仓库
- 没有生产C ABI/MnnExecutor、包schema迁移、每请求temperature/top_p/seed、线程安全中途取消、prepared代际、生产UTF-8/stop/usage和背压
- Linux harness期限/日志轮询与子进程终止仅是开发保护；不能移植为Android强杀线程。输出streambuf上限不限制MNN全部内存；短token输出可能截UTF-8，harness拒绝该结果
- 16KiB LOAD对齐不证明16KiB设备加载、整APK依赖闭包或运行正确；没有Android模型运行、设备性能/内存/持续负载/后台生命周期记录
- 预转换路径保留导出器/原始模型revision未知，不强制重导出；自行导出才须闭合转换身份。Flutter/bridge/JDK/Gradle/AGP未锁，OpenCL/QNN/直接Hexagon未验证。T07-B/C与产品P1均未完成

下一步：复核候选来源/许可并闭合Android运行证据；保持已有预转换资产不变，将转换来源未知与运行输入身份分开记录。并行推进不依赖设备的T07-B契约/实现，保留阶段门槛，不用探针替代产品验收。

## 独立CI入口与最终本地复核

新增`.github/workflows/android-mnn-probe.yml`及`ci_verify.py/test_ci_verify.py`，明确下载固定MNN源码、经size/SHA1/SHA256核验的NDK r30和五个固定模型文件；CMake本身不联网。Ubuntu24.04作业分别运行Linux真实六场景与Android交叉构建，不执行Android二进制。此处为发布前本地复核；首次GitHub运行及修复另记下节，不能用本地结果代替。

最终主审执行`python3 -m unittest discover -s scripts/android_mnn -p 'test_*.py'`为34项通过（19项探针本体加15项CI辅助），退出0；工具包版本检查及实际ELF检查均退出0。三段LOAD均16KiB对齐，GNU_RELRO起点3586128、长度67504、结束3653632（0x37c000）也满足16KiB对齐；拒绝可写可执行段/可执行栈和额外动态依赖。原生依赖只有libandroid/libc/libdl/liblog/libm。

CI脱敏上传采用固定JSON清单。只有完整构建身份、全部必需报告、六场景及预期超预算负例一致时才允许成功；CI源码不洁净或缺少证据会保留失败状态并返回失败。原始日志、正文、token序列、模型、SDK和二进制不上传。ZIP提取覆盖重复成员、链接祖先及完整链接图逃逸负例；官方NDK完整包的10134成员/39链接已实际提取验证。

未剥离Android开发CLI为88,347,296bytes；在独立副本执行NDK`llvm-strip --strip-unneeded`后为3,640,072bytes，SHA256 `88a64d344aed9745c0b13fe9fb1e3921bd5ca1f2da0351c933c8a0cb961f6529`。该数字只描述CPU原型ELF，不含Flutter/Rust/App资源/模型，也不是APK体积或已发布产物；原始验证二进制未改。

## 首次CI配置失败与最小修复

源码`e1ecc6e84464c41039ffe275a159c78cffe80470`的[Android运行36959563179](https://github.com/Naza3/Nexa/actions/runs/36959563179)在作业启动前失败，实际jobs为空；没有模型或Android编译结果。官方actionlint 1.7.7独立复现第29行job级env使用`runner.temp`非法，退出1；该上下文在此位置不可用。改为已忽略的`github.workspace/build/t07a/inputs`后同一linter退出0。未读取到GitHub页面annotation，不把本地复现冒充服务端错误正文。

修复仅改变输入暂存位置，不更换SDK/模型或放宽验收。设置`NEXA_ACTIONLINT`后的35项测试全部通过，包含还原非法上下文的语义负例；未提供该本地工具时语义测试明确skip，合法路径断言与其余测试仍运行。actionlint来自官方v1.7.7发布，archive SHA256 `023070a287cd8cccd71515fedc843f1985bf96c436b7effaecce67290e7e0757`与官方checksums一致。实际GitHub修复运行仍待验证。

## 修复后Android基础CI通过及证据复核

源码`c1114eeda3ad4a79587fd0c332f0af5422c85b02`、tree `87b8a470a4d0a155f2e0a4469aaf0d7c160b1004`的[Android CI36960074245](https://github.com/Naza3/Nexa/actions/runs/36960074245)于03:40:08 UTC成功，job`110691601352`。全部步骤通过；Linux真实六场景（含预期超预算拒绝）和重复一致为pass，Android arm64/API28交叉构建及ELF检查通过。报告明确`android_run=false`，不代表手机执行。

下载artifact `11207669112`，9,332bytes，SHA256 `acbddf611fc468eed3905c79284eed5122f2b7b21685342f3792f7e400e5cf26`，与GitHub digest一致；解压仅13份JSON。主审独立调用完整报告校验器并核对source commit/tree/clean、六项步骤成功、`evidence_verified=true`、全部模型文件与工具身份，均通过。证据保留`artifacts/verification/t07a-ci36960074245/`，没有二进制或模型发行包。

该CI Linux编译器为GNU13.3.0，Android仍Clang21.0.0；Android未剥离ELF为88,159,208bytes，SHA256 `aed123cc14b29176cbbc2d4f35d2af1d94c872b05082ebe48ee57aa717cfa802`。其LOAD/RELRO和系统依赖检查通过；构建宿主/路径不同导致二进制hash及调试段体积与本地结果不同，不声称字节级可复现构建。模型运行输入与构建身份可重取不等于最终二进制完全相同。Android真机、生产Executor、安全取消及APK仍待完成；同源码Windows回归另行跟踪。


同源码[Windows CI36960074287](https://github.com/Naza3/Nexa/actions/runs/36960074287)于04:11:59 UTC成功，job`110691633652`；全部生产构建/真实模型/存储调度/worker/HTTP/CLI/解压包及桌面bridge门槛通过。独立下载脱敏证据artifact`11209230562`，77,339bytes，SHA256 `ffdfde479592e9c755a0584d6ba52e841b04554df8e98f855e3247e330b143d0`与GitHub一致；50项索引文件的集合/长度/hash均复核，桌面acceptance为pass、external16项全true、native_window_tested=false。此轮仅复核回归报告，没有再次下载/交付Windows产品ZIP，不替代已交付75e458f目录版的待办原生UI验收。
