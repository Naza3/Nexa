# T07-B1：独立 MNN CPU C ABI

范围是 Android 引擎的原生请求实现，Linux 为开发验证。不是生产模型导入器、Rust Executor、完整 T07-B、APK 或真机准入。Windows 构建/llama 源码不变。

## 结构与所有权

- `include/nexa_mnn.h`：ABI 1，固定宽度字段、精确 `struct_size`、错误缓冲、独立取消对象
- `src/shim.cpp`：load → prepare（真实模板/分词/预算）→ 一次性 generate → owner 线程释放
- model、prepared、sampler、KV 仅创建线程操作；有未销毁 prepared 时不能释放 model，也不能再 prepare
- 唯一并发对象是一次性 atomic cancel；调用方必须保证请求及控制线程使用期间它都存活。取消不写任何 MNN 普通字段，不杀线程
- 回调和 userdata 只借用当次调用；prepare 拷贝消息所需 tokens/options/stops，不保存消息地址和 progress userdata；generate 有自己的 progress 参数
- prepare 成功保存的同一 token vector 直接用于 chunked prefill，不重复渲染/分词；用户历史由上层逐请求提供
- 每次 prepare 全新 greedy/topP sampler，显式 temperature/top_p/seed；`UINT32_MAX` 代表新随机熵；其余含 0 为固定 seed
- usage 计数是完整 prepared tokens 和已接受输出 token；包含 EOS/命中用户 stop 的 token，失败/取消保留已知数目
- token hook 在 owner 上执行；UTF-8 与跨 token stop 缓冲独立复制自 llama-shim，并保留同一 golden。文本 callback 最多 4 KiB，piece ≤1 MiB，stop ≤4×128 bytes
- 原始输入/渲染各≤1 MiB，消息≤4096，context≤2048，threads 1..2，chunk 1..128。当前真实证据使用 Linux threads=2/chunk=32，其他允许数值不是设备性能认证
- callback 返回 0 接受、1 取消、2 失败；其他值视失败；不允许抛异常、重入或无限阻塞。Rust 负责 panic 和背压取消唤醒

输入必须来自已经校验的私有运行配置与不可变模型租约。shim 检查严格 CPU/固定 Qwen3 元数据/禁用辅助图及缓存配置白名单，但不导入包、不验证调用方传入 artifact SHA 对应的磁盘权重，也不抵御同 UID 文件替换。hash/引用闭包、租约与路径所有权由后续 store 负责；本模块不能把候选升格为 production validated。

## 精确私有上游补丁

`../mnn-patches/lock.json` 锁定 MNN commit、依序 patch SHA256、13 个修改文件完整 before/after SHA256、policy/ABI 身份。补丁只作用于新建私有源码副本。

```sh
python native/mnn-patches/identity.py \
  --source /path/to/clean-MNN-3.6.1 \
  --destination /path/to/new-private-MNN
python native/mnn-patches/identity.py --source /path/to/new-private-MNN --verify-post
```

应用前要求精确 commit、无 tracked/untracked 改动、每个目标为普通文件且 preimage hash 一致；`git apply --check` 后应用，核对 postimage。副本保留上游已有的非修改 symlink，修改目标不允许 symlink；不会把修改写回原始 checkout。构建再次校验目标集合和全部 postimage。禁止 fuzz、在线下载或自动换版本。

补丁扩展仅在同步 owner 请求路径使用。无 hook 的上游 AR 路径保留作独立对照；hook 路径在接受 token 后只判一次 EOS，终止后不跑多余 forward、不积累 `generate_str`。load/template/tokenizer/embedding/forward 仍可能有不可抢占调用，取消必须等其返回；测试检查点样本不等于任意时刻或目标手机的 1 秒保证。

## 独立构建

Linux（开发验证）：

```sh
cmake -S native/mnn-shim -B /path/to/linux-build \
  -DNEXA_MNN_SOURCE=/path/to/new-private-MNN -DCMAKE_BUILD_TYPE=Release
cmake --build /path/to/linux-build -j2
ctest --test-dir /path/to/linux-build --output-on-failure
python native/mnn-shim/audit_build.py /path/to/linux-build /path/to/new-private-MNN
python native/mnn-shim/export_artifact.py /path/to/linux-build \
  --target x86_64-unknown-linux-gnu --compiler "$(c++ --version | head -1)"
```

Android（NDK r30 `30.0.16248370`、API 28、arm64，仅交叉构建）：

```sh
cmake -G Ninja -S native/mnn-shim -B /path/to/android-build \
  -DNEXA_MNN_SOURCE=/path/to/new-private-MNN -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
  -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-28
cmake --build /path/to/android-build -j2
```

Android 导出再传 NDK 下的 `--cxx-static`、`--cxxabi-static`、`--unwind-static`、`--builtins-static` 精确 archive 路径；输出 `artifact/artifact.json` 包含 target/compiler/header/commit/patch/policy/archive SHA256、完整静态链接顺序、system libraries 和 NDK revision/API。MNN 单一 archive 已合并本 CPU 构建全部对象；Rust 不隐式编译、下载或找猜测路径。生成器从 CMakeCache/实际编译器/NDK source.properties 核对来源，不把目标三元组当设备证据。

## 测试

生成可重跑的固定研究配置（核对候选五文件和模板hash）：

```sh
python native/mnn-shim/prepare_research_config.py \
  --model-root /path/to/fixed-candidate --output /path/to/private-runtime.json
```


- 无模型 `mnn-request-test`：ABI/null/error-header、合成 topP/greedy/seed logits
- `mnn-stream-test`：中文/emoji跨 byte、stop前缀/交叠、非法/残缺UTF-8、4KiB分片与piece上限
- `mnn-request-test CONTROLLED_CONFIG.json [REPORT.json]`：固定候选中英/空/system/多轮、EOS/max1/stop、预算等于/超1、seed A-B-A；load/prepare/prefill/decode阶段跨线程取消及恢复；callback拒绝/重入、错误线程、一次性/lifetime；hook token-ID 与无hook路径对照
- `tests/privacy_canaries.py BINARY CONFIG`：8个输入负例及真实中英/多轮、取消、callback拒绝/恢复的独立进程canary矩阵；输出marker必须实际被callback观察，检查stdout白名单/stderr空及load内宿主线程日志保留
- ASan/UBSan仅对纯 stream 测试运行；不能宣称完整 MNN 内存安全通过

运行配置应由可信候选 metadata 重建，使用 CPU/high/low、async=false、chunk=32、threads=2、context=2048、greedy load default、全部 cache/mmap=false、speculative_type为空，并含原始模板和 `enable_thinking=false`。测试工具不向默认日志写 prompt/回答。详细实际结果与剩余门禁见 `VERIFICATION.md`。

## 日志边界

所有 MNN/Express/llm/shim 编译单元设置 `NEXA_MNN_SILENT_LOGS=1`。宏在最高优先分支无格式化/参数求值/副本，禁止 LLM_LOG_TO_STRING、JINJA_DEBUG、DUMP_PROFILE_INFO。私有 patch 同时处理直接 tokenizer/config/unicode/embedding/omni/speculative/core 输出。无全局 stdout/stderr 重定向、iostream rdbuf 替换或运行期日志 callback。

shim 错误仅固定字面量，最多512 bytes，不转发 `what()` 或 `getLog()`。审计脚本是具体编译闭包词法检查的辅助证据，不能替代跨头文件/文件 dump 可达性判断或真机 logcat。系统崩溃诊断不是可控应用日志保障。
