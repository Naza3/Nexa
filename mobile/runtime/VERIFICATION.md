# T07-B1 Rust adapter 验证（2026-10-02 UTC）

状态：本目录实现和Linux研究/Android完整链接通过；生产模型准入、Android设备运行、Executor/store/UI不在本片完成范围。

## 固定身份

- Linux本轮实际编译器为Debian GCC14.2.0；另允许Ubuntu GCC13.3.0 CI构建profile，该profile尚待CI真实native/Rust门禁，不以允许列表充当验证证据
- Rust/Cargo 1.98.1；独立workspace只有mnn-adapter，metadata实际确认workspace_root为mobile/runtime
- MNN `d407447ed56c4121a11ccbd266dc184ca1ead0c2`
- patch set `43cc33146e2036ff452bd02d5ec352bb099d143ed4a4cdeb6ff55335987f9ce0`
- policy `ea06621b78e67e58f97f98951b26db0a8a893ded4112da3e5a762b98566fa328`
- frozen ABI1 header `40d1a79df99f5200d92e689baf791d5105d2da71d521fcd885a2807f0b916b8e`
- Linux shim archive `347849aa93bc5c222dbc7ed27e09775fc5e001e89bde11a867645c12655a49cc`；MNN archive `24d727baca3d63dc6e70589eef09910ec93142a2a0146ef0db0e19520831b1d6`
- Android shim archive `dbf3bdbd3e272735ba084a5543efd0be8329763b8a458b6ea1a4b6e74e49f5e0`；MNN archive `56b93cceed95a8e6cfdfb34814d779589accfbb09e6fa01f7caef7063b7a638b`
- 完整native manifest指纹：Linux `53eab05ec35465082f784af6e305635376e9e12f8284cc4d208d1f5f40448450`；Android `be2c62705f861ef6115d8a035fd1482662ccda0d11e8fbd642b4ca5b81676a72`。Linux实际build_identity输出与manifest SHA256一致；原始JSON包含全部archive/header/compiler/target字段，格式变化也保守改变指纹
- 候选五文件逐个size/SHA256复核通过；研究锁规范JSON摘要 `8d43d63204f753fd4a0b6b9089209f5641e206d5314c02a54186f17e556c1720`，不冒充未来生产包身份

## 实际运行及结果

以下命令均exit 0（负例runner内部要求被测cargo非零）：

1. `cargo metadata --manifest-path mobile/runtime/Cargo.toml --locked --offline --no-deps --format-version 1`：独立成员/根确认；根Cargo.toml/Cargo.lock `git diff --exit-code`无变化，两个lock所有共享依赖版本相同
2. `cargo test --manifest-path mobile/runtime/Cargo.toml --locked --offline`：6单测+4 compile-fail文档测试通过；真实测试默认显式ignored，未当作真实运行证据
3. `python3 mobile/runtime/scripts/test_real_model.py --model-dir VERIFIED_CANDIDATE --config CONTROLLED_RUNTIME`：真实测试显式运行通过，包含load取消/回调panic、callback嵌套load Busy、seed0 A→B→A、中文system/多轮、正常stop与取消/失败区分、精确模板预算/超1、源字符串已销毁后生成、template/tokenize/prefill/decode取消与每次恢复、基于phase的跨线程取消、text/progress panic失败及恢复、max1、close幂等/关闭后拒绝
4. Linux和Android分别 `cargo clippy ... --all-targets -- -D warnings`：通过；fmt --check通过
5. Linux `test_artifact_gate.py`：8项拒绝通过（ABI/target/patch/header/silent/compiler/污染archive/缺显式输入）。Android另加NDK版本/API错配，共10项拒绝通过
6. `tests/abi_layout.c`：Linux C11 `-Wall -Wextra -Werror`编译并运行；Android NDK clang API28编译对象通过，与Rust结构size/offset断言一致
7. `cargo test --no-run --target aarch64-linux-android --manifest-path mobile/runtime/Cargo.toml --locked --offline`：最终两个测试ELF与build_identity示例ELF完成Rust→shim→MNN→静态libc++/abi/unwind/builtins链接，非仅cargo check。NDK r30/30.0.16248370、Clang21、API28
8. `readelf -lW/-dW`：三个ELF均ARM aarch64、解释器/system/bin/linker64，全部LOAD对齐0x4000且GNU_RELRO末端16KiB对齐，DT_NEEDED仅libm.so/libdl.so/libc.so

最终Rust链接显式设置max-page-size=16384与common-page-size=16384，纠正早期仅检查LOAD对齐而GNU_RELRO仍4KiB的问题；不把早期LOAD结果算作完整16KiB门禁。

初次Android门禁正确拒绝NDK的libatomic.a纯注释兼容占位；native导出改为真实libclang_rt.builtins-aarch64-android.a（manifest名clang_rt_builtins）后重试成功，没有放宽非ELF archive校验。

## 开发环境原始证据

本轮未把模型或构建大文件纳入仓库。原始日志位于开发机 `/workspace/shared/`：

- `nexa-mnn-rust-test.log`、`nexa-mnn-rust-real-test.log`
- `nexa-mnn-rust-build-negative.log`、`nexa-mnn-rust-android-build-negative.log`
- `nexa-mnn-rust-android-link.log`、`nexa-mnn-rust-android-elf.log`
- `nexa-mnn-rust-metadata.json`、`nexa-mnn-rust-build-identity.log`

## 限制

- 没有Android真机执行，不授予生产validated、取消时延目标、Android日志/内存稳定性或APK完成结论
- 直接config入口仅可信研究组合；真实hash runner不提供生产资产闭包/TOCTOU/私有目录租约机制，后续store承担
- callback panic通过catch_unwind转明确失败；宿主panic hook仍会执行，panic=abort不能恢复。无全局hook替换
- callback内Drop另一个模型/Prepared按不可重入门禁拒绝并泄漏，须保留句柄至callback返回后close；这是保守失败边界，不是无泄漏承诺
- 未stage、commit、push；native目录由独立owner维护，Rust本片没有修改它们或根Cargo文件
