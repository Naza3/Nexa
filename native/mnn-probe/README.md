# T07-A MNN CPU 开发探针

这是独立 C++ 命令行原型，作为 Android/MNN 底层第一切片；不是最终 Android 产品、MNN Chat 功能对标交付、生产 C ABI、MnnExecutor 或 APK。Windows llama 和公共 Rust 协议不变。

## 固定边界

- MNN 3.6.1 完整 commit `d407447ed56c4121a11ccbd266dc184ca1ead0c2`；构建要求外部 Git checkout 精确且全洁净。不下载源码、工具链或模型，不修改上游，补丁集为空
- 静态 MNN + LLM，CPU、text-only、同步计算；关闭 HTTP resource、OMNI、视觉/音频、OpenCL、QNN、Hexagon 等后端，KleidiAI/SME2 也不进入首轮组合；MNN_BUILD_OPENCV=OFF只禁用OpenCV API扩展，上游source/cv仍参与编译
- 使用 MNN 实际 `createLLM/load/apply_chat_template/tokenizer_encode/response(vector<int>)/destroy`。同一个线程拥有和释放全部 Llm 资源，无线程间句柄或状态写入
- 1–4线程、最多16条合成消息、输入/渲染64KiB、逻辑context最多2048、输出1–64 token；精确预算消费同一 token vector，不重复模板、不截断输入
- `sampler_type=greedy` 在 load 前固定。没有每请求 temperature/top_p/seed 接口，不能把 `set_config` 当作 sampler 重建
- 没有线程安全取消。Linux harness 的有界子进程终止只用于独立开发验证，不可用于 Android App 线程；native prefill 本身没有硬时间上限
- 输出streambuf有256KiB上限，溢出后丢弃并在短token循环结束时报错；不声称这是 MNN 内部 generate_str/KV/权重/工作区的内存上限，也不声称生产流式/背压已完成
- 输出 token 计数包含 MNN 的终止 token，不能直接当作已冻结公共usage。短token上限可能截在UTF-8序列中，harness拒绝无效UTF-8结果；生产增量UTF-8/stop处理留待T07-B
- 原生异常可以使无异常构建的MNN abort，catch不是崩溃隔离。该工具仅接受harness生成的受信运行配置，不是通用不可信模型导入器

## 本地离线构建

已有工具链须由调用者提供；不在此安装SDK或接受协议。运行输出目录放在仓库忽略的build/artifacts或外部工作目录。

```sh
cmake -S native/mnn-probe -B build/mnn-linux -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DNEXA_MNN_SOURCE=/existing/MNN
cmake --build build/mnn-linux --target nexa-mnn-probe -j4
ctest --test-dir build/mnn-linux --output-on-failure
python -m unittest discover -s scripts/android_mnn -p 'test_*.py'
```

Android 仅交叉编译原生CLI（须已有获准NDK）：

```sh
cmake -S native/mnn-probe -B build/mnn-android -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DNEXA_MNN_SOURCE=/existing/MNN \
  -DCMAKE_TOOLCHAIN_FILE=/existing/android-ndk-r30/build/cmake/android.toolchain.cmake \
  -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-28
cmake --build build/mnn-android --target nexa-mnn-probe -j4
```

不得在宿主直接执行Android二进制或把交叉编译/ELF检查等同真机通过。`nexa-mnn-build-Release.txt`记录实际编译器/系统/处理器/API；`--identity`报告编译目标，不推断设备身份。Linux `MNN_USE_SSE/MNN_AVX2`仍为上游默认ON，仅开发宿主组合，不承诺任意x86 CPU可用。

## 固定公开候选模型

`scripts/android_mnn/candidate-model.json`只是T07-A研究输入锁，不是生产模型包schema。来源 [taobao-mnn/Qwen3-0.6B-MNN](https://huggingface.co/taobao-mnn/Qwen3-0.6B-MNN/tree/34dfccda1187ded6e07ea06426da576b0b793c6b)。已锁5个文件大小/sha256、revision、模板hash。元数据表明embedding引用同一weight，严格核对已知偏移/alpha范围。

上游预转换包没有锁定精确exporter commit和原始Qwen模型revision，本探针不补造这些身份；可重复导出门槛仍未通过。模型权重和生成二进制不入源码仓库。未来分发须包含依赖许可/NOTICE闭包；本轮不分发二进制，Unicode数据来源说明仍待补。

```sh
python scripts/android_mnn/run_probe.py \
  --probe build/mnn-linux/nexa-mnn-probe \
  --model-dir /existing/fixed-five-file-model \
  --out-dir artifacts/verification/mnn-short-en \
  --case short-en --threads 2 --max-new-tokens 16 --logical-context 512
```

输出目录必须不存在。可选合成case：short-en、short-zh、multiturn、empty。输入目录只能含固定5文件及可选不读取的README.md/.gitattributes/LICENSE；拒绝额外context、子目录、文件symlink、hash漂移。不要并发修改源模型；当前没有生产级TOCTOU防护或不可变导入存储。

运行配置在临时私有目录重建，不改模型源config；MNN的llm_config合并后再次验证CPU/greedy/上下文配置。禁用context_file自动加载，加载后显式重新应用已核验Jinja上下文`enable_thinking=false`；模板由MNN执行，Python独立golden只作验证，绝不代替native模板结果生成输入。未实现的路径/配置一律不从源config透传。

`report.json`可用于脱敏开发报告，含模型/二进制hash、CPU配置、目标编译身份及时间，不含完整路径或正文。`synthetic-result.json`含合成渲染文本、真实token序列和合成生成正文；`private-native.log`可能有上游完整路径，仅本地私有验证，不整体上传。计时仅单次功能观测，不是设备性能基线；native_prefill/decode为上游计数。

harness启动后以50ms轮询检查180秒期限（可选1–600）和1MiB日志软阈值，进程退出后也复查日志大小。这不是硬限制的pipe sink，轮询之间日志可超过阈值。超限kill后最多两次各5秒等待回收；未确认回收明确报`development_cleanup_unconfirmed`。这些操作仍只属于Linux独立进程，不能宣称达到Android安全取消验收。

## 后续门槛

T07-B仍需生产C ABI/模型包迁移、真实每请求采样、prefill/decode中途原子取消、prepared代际、精确stop/UTF-8/usage、单一输出账本及异常恢复。T07-C需Android App生命周期；MNN Chat能力对标另按产品计划推进。OpenCL/QNN/Hexagon、真机运行/内存/持续负载与导出器身份均不能由本探针代替。
