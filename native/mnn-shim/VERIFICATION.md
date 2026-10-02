# T07-B1 原生 CPU 验证（2026-10-02）

状态：原生请求切片已实现并完成下列 Linux 开发验证、Android arm64 交叉构建；不等于完整 T07-B 或 Android 真机/产品准入。仅改 `native/mnn-shim/`、`native/mnn-patches/`；未修改 Windows 或共享原始 MNN checkout，未提交/推送。

## 固定身份

- 上游 MNN 3.6.1 commit：`d407447ed56c4121a11ccbd266dc184ca1ead0c2`
- patch-set SHA256：`43cc33146e2036ff452bd02d5ec352bb099d143ed4a4cdeb6ff55335987f9ce0`
- policy SHA256：`ea06621b78e67e58f97f98951b26db0a8a893ded4112da3e5a762b98566fa328`
- C ABI header SHA256：`40d1a79df99f5200d92e689baf791d5105d2da71d521fcd885a2807f0b916b8e`
- 完整3个patch SHA、13文件before/after SHA在 [lock.json](../mnn-patches/lock.json)。独立全新副本精确重放及postimage验证退出0；原始checkout `git status --porcelain`仍为空
- 输入：`scripts/android_mnn/candidate-model.json` 固定五文件全部大小/SHA256重新流式核对；Qwen3-0.6B MNN revision `34dfccda1187ded6e07ea06426da576b0b793c6b`；模板 SHA `87a2728cb8dc9fe424d624542f6060ec05a1d285ebbec578bb078900e33396b5`。这是公开预转换研究资产，不是 production admitted；exporter/original revision 未知边界不变
- Linux：GCC `c++ (Debian 14.2.0-19) 14.2.0`、Release、2线程、CPU/high/low、sync、context2048/chunk32
- Android：NDK r30 `30.0.16248370`、Clang21、API28、arm64-v8a、Release；没有设备运行

最终 shim archive SHA：

| 平台 | `libnexa-mnn-shim.a` SHA256 |
|---|---|
| Linux | `347849aa93bc5c222dbc7ed27e09775fc5e001e89bde11a867645c12655a49cc` |
| Android | `dbf3bdbd3e272735ba084a5543efd0be8329763b8a458b6ea1a4b6e74e49f5e0` |

全部依赖库hash/链接顺序由各build目录 `artifact/artifact.json`记录；Android显式导出真正的 `libclang_rt.builtins-aarch64-android.a`，不把r30的ASCII `libatomic.a`兼容占位当真实archive。导出前重新验证private source、全target宏/头闭包；compiler、NDK/API由实际CMake/工具链核对，不信任命令行自报身份。

## 实际测试与命令

1. `cmake --build /workspace/shared/nexa-mnn-shim-linux -j2`：退出0
2. `ctest --test-dir /workspace/shared/nexa-mnn-shim-linux --output-on-failure`：3/3（stream、ABI/synthetic sampler、logging-macro），退出0
3. `mnn-request-test /workspace/shared/nexa-mnn-shim-runtime.json /workspace/shared/nexa-mnn-shim-comparison.json`：退出0；中英、空内容、system/多轮、EOS、max1、用户stop、精确预算等于/超1、seed42 A→seed999不同参数B→A恢复相同结果；ABI版本/sizeof/null/溢出/NaN/未知role/保留位、错误线程、callback重入、prepare存活期间释放限制、重复generate、拒绝callback后恢复
4. load检查点0/2/3/4/5/6跨线程cancel、template/tokenize阶段cancel、成功prefill32-token后cancel、decode接受token后cancel，均安全返回并重新生成成功；普通native推理失败使model faulted，需销毁/重载，不伪装恢复
5. 原始未打补丁 `nexa-mnn-probe-linux-target/nexa-mnn-probe` 使用同一受控配置和固定合成请求运行，退出0。`tests/compare_upstream.py`对照退出0：完整模板字符串、21个prompt tokens、12个output token IDs、shim字节输出、usage全部相同。测试内另比较hook的实际accepted token IDs与无hook路径，断言`generate_str`为空，预算末token不额外forward
6. `tests/privacy_canaries.py BINARY CONFIG`：退出0；8负例+真实成功矩阵全部通过，见下节
7. `g++ -std=c++17 -fsanitize=address,undefined -fno-omit-frame-pointer -UNDEBUG tests/stream_buffer_test.cpp ...`及`ASAN_OPTIONS=detect_leaks=0`执行：退出0。包含独立审查发现的“stop之前残缺UTF-8不能吞掉”golden；仅改MNN私有副本，不改Windows copy
8. `audit_build.py`：Linux 314个编译单元/242个实际依赖头、Android445/220，零未分类直接sink；退出0
9. 配置负例 `-UNEXA_MNN_SILENT_LOGS`、`-DNEXA_MNN_SILENT_LOGS=0`、`-DJINJA_DEBUG=1`、分开参数`-D DEBUG_IMAGE`全部拒绝。审计也按shlex归一化检查每个target参数，不能用后置`-U`蒙混；shim自己编译断言silent宏为1
10. Android `cmake --build ... -j2`：退出0；`llvm-readelf -l -d mnn-request-test`的所有LOAD对齐为`0x4000`，NEEDED只有系统`liblog/libm/libandroid/libdl/libc`，无shared libc++。仅证明交叉构建/ELF，不证明手机执行

## 取消响应样本

计时起点在独立控制线程实际写atomic之前，终点是C ABI返回之后（包括partial-load RAII释放）。下列单轮Linux样本，非最大值/分位数、非任意kernel时刻、非手机1秒承诺：

| 检查点 | cancel→安全返回 ms |
|---|---:|
| load进入/构造前 | 0.072674 |
| load runtime之后 | 0.125644 |
| load tokenizer之后 | 39.5944 |
| load embedding之后 | 23.9734 |
| load Module之后 | 29.6774 |
| load strategy/clone之后 | 34.9804 |
| prepare/template | 0.056570 |
| prepare/tokenize | 0.055664 |
| 完成32-token prefill chunk | 0.097147 |
| decode接受token后 | 0.054167 |

当前调用不可抢占：加载Module、模板、tokenizer、embedding、forward仍需自然返回。不能由这些检查点样本推导卡死kernel的停机保证；不会强杀线程或异线程释放模型。

## 日志与dump审计边界

8负例：损坏config、损坏tokenizer、损坏graph、模板语法错误、缺失tokenizer、缺失graph、非法backend、非法辅助图字段。每例不同synthetic path/config marker，检查stdout精确固定集合、stderr空；宿主另一线程在load回调期间确实写出自己的固定日志，排除库全局吞日志。

真实成功矩阵分别把唯一marker送入英语/中文/多轮prompt，并要求三个不同输出marker实际出现在text callback里；随后prefill阶段取消、完整decode marker被callback观察后跨线程取消、另一完整marker被观察后callback失败，并恢复同一生成。外层仅允许固定测试标记及数字耗时，要求无任何CANARY和空stderr。不是以“模型没产生marker”冒充通过。

实际依赖头检查分类：RapidJSON示例中的printf/fprintf是块注释；MNNDefine的其他sink分支被silent最高优先分支排除；llm/omni头中的cout仅默认参数（shim不调用response，generate_init传nullptr）；tokentree输出处于literal `#if 0`；JINJA_DEBUG由构建拒绝。每个文件SHA与分类记录在build目录`logging-audit.json`。

文件dump额外人工边界：llm/mtp的张量ofstream处于`DEBUG_MODE==3`，锁定源码为0；Omni图dump在禁用的DEBUG_IMAGE且plain Llm文本profile不构造Omni；HTTP下载路径由LLM_SUPPORT_HTTP_RESOURCE=OFF且text profile不可达；speculative_type为空阻止dflash/eagle/mtp策略进入；CPU不设置后端编译缓存，mmap/KV/prefix cache关闭。FileLoader::write是明确文件保存API，不是默认日志；Express内部保存到NetT/byte vector不是文件dump。独立demo apply_template的result.json属于保守compile_commands库存中的未链接应用目标，不属于shim执行路径。

宏日志参数审计未发现本text/CPU路径依赖的必要副作用；移除的是诊断读值/格式化，eagle调试中的readMap/token decode不属于启用策略。该源级+canary证据限定锁定输入和配置；不声称形式化证明所有C++/系统库可达性。Android logcat、系统崩溃诊断及新设备/新profile仍未验。

## 尚未完成

- Android手机CPU运行、后台生命周期、logcat canary、重复装卸内存趋势/长稳/取消最大值与分位数
- 完整MNN ASan/UBSan/LeakSanitizer；当前仅纯stream测试，LSan未启用
- 原生回调的core 256KiB账本、10秒无消费进展、执行器终态与drop/close；属于后续B2，callback拒绝测试不冒充完整背压系统
- 生产store的包引用闭包/导入事务/lease/TOCTOU/production evidence；native接受的是调用方已经校验的研究运行路径，不自行证明artifact SHA对应磁盘内容
- OpenCL/QNN/Hexagon和其他模型/配置；本CPU证据不能复用为其准入

## 宿主证据位置

可重跑入口均在本目录；本次原始输出保留在`/workspace/shared/`：`nexa-mnn-shim-ctest-final.log`、`nexa-mnn-shim-real-final.log`、`nexa-mnn-shim-privacy-final.json`、`nexa-mnn-shim-upstream-comparison.json`、`nexa-mnn-shim-android-elf-final.txt`及两平台build/artifact目录。大模型、源码副本、二进制、合成详细report未加入仓库。
