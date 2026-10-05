# Linux → Windows x64 私有测试包

这是独立的交叉构建与测试打包入口。`package_windows.py` 和 `package_desktop_windows.py` 的原生 Windows / Visual Studio 路径不变。此路径不使用 GitHub Actions、不下载或安装工具、不运行 Windows EXE、不替代目标机验收，也不修改推理/API/桌面运行时行为。

## 范围与前置条件

- 仅供本次已授权用户的私有开发测试，不是公开 Release；不自动推断用户的许可资格或额外分发权限
- 已核验并在适用条款下取得的 Rust 1.98.1 Linux host + `x86_64-pc-windows-msvc` 标准库，Clang/LLVM、CMake、Ninja、Microsoft SDK/MSVC headers/libs 和官方 Release x64 CRT；安装/登录/接受条款不由此脚本执行
- C/C++ 必须是 Clang、MSVC frontend/ABI、精确 `x86_64-pc-windows-msvc` target、Release `/MD`。不把 Clang 身份改成 MSVC
- 固定 CPU 基线要求 SSE4.2、AVX、AVX2、FMA、F16C、BMI2；AVX512 关闭。核对锁定 ggml-cpu 目标的实际 `/arch:AVX2` 和对应定义。不能宣传任意 x64 CPU 均兼容
- 主项目必须有真实 Git commit/tree/status；显式 sparse checkout 如实记录缺失路径数和已物化源文件 hash。`vendor/llama.cpp` 必须是锁定 `2149c00f4442dc59302e134a02e4c99d5f7ed9fc` 的真实干净 Git checkout
- aria2 必须从最终相同 source commit 真正重新构建，其原 source-lock、三补丁、原许可、对应源码和二进制清单均继续核验；不得改旧组件 manifest 冒充同源
- 本包不含模型权重，不自动安装 WebView2，不运行网络下载验收

## 构建顺序

先冻结本次构建源码，记录实际 commit 与 dirty 状态，再捕获源收据。收据路径须在源码树外或已明确忽略的构建目录，且不存在；后续打包会再次比较源码身份。

```sh
python3 scripts/package_windows_cross_test.py --capture-source /absolute/evidence/source-receipt.json

cmake -S native/llama-shim -B /absolute/build/native -G Ninja \
  -DCMAKE_TOOLCHAIN_FILE=/absolute/tools/windows-msvc.cmake \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL \
  -DAIR_NATIVE_PROFILE=linux-clang-cl-msvc \
  -DGGML_SSE42=ON -DGGML_AVX=ON -DGGML_AVX2=ON -DGGML_FMA=ON \
  -DGGML_F16C=ON -DGGML_BMI2=ON -DGGML_AVX512=OFF
cmake --build /absolute/build/native --target air_llama --parallel 2

export AIR_NATIVE_PROFILE=linux-clang-cl-msvc
export AIR_NATIVE_DIR=/absolute/build/native
export CARGO_TARGET_DIR=/absolute/build/cargo
cargo xwin build --locked --release --target x86_64-pc-windows-msvc \
  -p runtime-cli -p runtime-worker -p xtask
```

工具链文件必须显式设置 `CMAKE_C_COMPILER_TARGET` 与 `CMAKE_CXX_COMPILER_TARGET` 为上述 MSVC target，并使用真实 SDK/CRT。CMake 编译 Windows 目标宏与 `/MD` 探针，不在 Linux 运行该探针。Cargo-xwin 的工具/SDK版本与缓存也须隔离并记录。

前端使用已有 package-lock/npm 流程，先执行 `npm ci`、既有测试/typecheck/lint 和 `npm run build`；本路径不迁移包管理器。随后以现有桌面 Cargo.lock 构建独立 `apps/desktop/src-tauri/Cargo.toml` 的 Windows Release。实际命令、退出码、工具版本和日志独立保存；准备性/探索构建不能充作最终源码身份的运行验收。

## CRT 与工具链来源记录

打包要求 `--toolchain-provenance` 的完整实际工具链记录，并要求 `--crt-provenance` JSON：

- `schema_version: 1`
- `archives`：每项包含 `path`、官方 Microsoft HTTPS `url`、`sha256`、`size_bytes`，保留包 ID/版本
- `dlls`：每项包含 `path`、`sha256`、`size_bytes`、`archive_sha256`、精确原 ZIP/VSIX `package_path`
- `licenses`：每项包含原文 `path`、`sha256`、`size_bytes`、`source_url`。按[十文件许可整合契约](decisions/0026-lossless-license-bundles.md)，UTF-8 原文汇入文本；DOCX/PDF或其他已支持的非 UTF-8 Microsoft 原件保持原字节、原格式，独立存为 `licenses/ORIGINAL-<原文件名>`。此时本层 `THIRD_PARTY_NOTICES.md` 原字节汇入文本并由索引绑定，腾出文件位置；runtime仍自包含，完整桌面仍受至多10份许可文件的硬门槛约束。不转码、不删原件，也不另建压缩包。

工具链记录中的目录/元数据 hash 或 size 验证局限必须原样保留，不得用自算 hash 宣称上游校验已通过。打包器重新核验 archive、原 member 和 DLL 字节关系，只复制实际普通/delay import 闭包需要的 Release DLL。每种实际复制字节须通过指定 `osslsigncode verify` 的签名、时间戳、CRL 检查；无忽略/跳过验签参数。manifest 保留工具/信任根/日志 hash、警告和 Windows 策略不等效标识。Windows `Get-AuthenticodeSignature` 明确未执行。

## 生成可测试 ZIP

全部输入路径都必须指向本次真实构建产物。工具链、签名根和源码收据应为前面已核验的同一组输入。

```sh
python3 scripts/package_windows_cross_test.py \
  --source-receipt /absolute/evidence/source-receipt.json \
  --native-dir /absolute/build/native \
  --runtime-exe /absolute/build/cargo/x86_64-pc-windows-msvc/release/ai-runtime.exe \
  --worker-exe /absolute/build/cargo/x86_64-pc-windows-msvc/release/ai-runtime-worker.exe \
  --acceptance-exe /absolute/build/cargo/x86_64-pc-windows-msvc/release/nexa-acceptance.exe \
  --desktop-exe /absolute/build/desktop/x86_64-pc-windows-msvc/release/nexa-desktop.exe \
  --component-dir /absolute/build/download \
  --crt-provenance /absolute/tools/crt-provenance.json \
  --toolchain-provenance /absolute/tools/toolchain-provenance.json \
  --llvm-readobj /absolute/tools/bin/llvm-readobj \
  --osslsigncode /absolute/tools/osslsigncode/bin/osslsigncode \
  --authenticode-ca /absolute/tools/licenses/Microsoft-Root-2011.pem \
  --authenticode-tsa-ca /absolute/tools/licenses/Microsoft-Root-2010.pem \
  --output /absolute/delivery/new-cross-test-output
```

输出目录必须不存在。ZIP 内有 `desktop-windows/`（完整 App/runtime/aria2）与独立 `acceptance-tools/`；用户在 Windows 启动 `desktop-windows/nexa-desktop.exe`。各层继续使用实际消费者要求的 `nexa-desktop`、`nexa-runtime`、`nexa-download` 产品值；交叉测试标识写附加字段，不改变消费者契约。

打包会在隔离的临时 Rust harness 中直接引用原 `layout.rs`、`selection.rs`，并调用生产 `download-engine::identity::verify_component`；exact 直接依赖、offline 构建、原源文件 hash 与结果记录在清单。该检查只证明 Linux 上的实际文件身份消费者接受这些字节，不证明 Windows 路径/共享句柄语义、窗口、下载或推理已运行。普通与 delay PE import、闭合文件清单、每文件大小/SHA、ZIP 字节与许可证另行核验。

## 逻辑检查

```sh
rustc --edition=2024 --test crates/llama-adapter/native_identity.rs -o /absolute/build/native-identity-tests
/absolute/build/native-identity-tests
python3 -m unittest discover -s scripts -p test_windows_cross_package.py -v
```

这些逻辑测试与 Rust/C++ 编译、静态 PE 检查、Windows 执行、真实模型、独立目标机验收分别记录，不能互相替代。
