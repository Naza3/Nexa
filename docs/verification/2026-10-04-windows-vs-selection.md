# Windows 既有 Visual Studio 选择修复验证

日期：2026-10-04。记录性质：提交前验证快照，以下提交/组件状态仅描述本记录形成时点，不代表其后提交或重建结果。任务：W02 本地 Windows 构建解阻。状态：脚本逻辑检查已完成，原生 Windows 构建/运行待验证。

## 范围与来源

基线为 `Naza3/Nexa` 的 `0d5b1dd5e77807239d8af99d39755ee381b2fae9`；本快照记录时，候选尚未形成新提交。修改范围为 `scripts/package_windows.py`、`scripts/test_windows_package.py` 及[构建锁](../build-lock.md)、[当前状态](../../PROJECT_STATE.md)、本记录；文档采用新增当前覆盖段，旧验证历史保持原样。截至本快照记录时，未提交、推送、运行 Rust 构建或触发 GitHub Actions。

通过 GitHub 精确提交树只读核对的原始 Git blob：

- `docs/build-lock.md`：`865ef1406d993ff49fa902532df104fc69b96411`
- `PROJECT_STATE.md`：`ae8e5dfe2a19f089119fdd5f2dfb428b19f5315b`
- `PROJECT_INDEX.md`：`14911093c0b01d39fae792b2ead3c3f72cef54a4`，只读未改
- `AGENTS.md`：`d6057e396f486adcfc974030d8766175f4f2b790`，只读未改

源文件来自局部只读材料目录，没有 `.git`；本地材料的上述四文件均比原始 blob 多一个末尾换行。文档候选以恢复原始字节并核对 Git blob 后的内容为基线，只在独立副本追加当前覆盖段，原材料未改，未初始化仓库。文件存在/链接检查结合该精确提交的完整树进行，不将局部材料误称完整 checkout。

## 实现与检查

候选优先复用最高版本的既有完整正式 VS2026/VS2022，包含 Build Tools 和仅安装 side-by-side 工具集的实例；逐个验证，坏实例回退其他既有可用实例。排除预览、不完整和未知版本；缺 C++/SDK/CRT 时给“修改已有 VS”说明，完全没有可用环境时才给 VS2022 Build Tools 手动安装兜底。没有自动下载、安装或许可接受。

生成器绑定实际 VS17/VS18，CMake 同时固定实例、x64 与精确 v143/v145 工具集版本；MSVC 工具和 Release CRT 来源受同实例约束。旧开发命令行状态只在子进程环境清理，工具定位避免解码非 UTF-8 `where.exe` 输出并拒绝当前目录同名程序遮蔽。原生/Cargo 缓存按实例路径、生成器、工具集身份共同隔离；旧缓存保留，已有缓存不匹配时明确失败。

实际验证命令及结果：

| 执行方/级别 | 命令 | 结果 |
| --- | --- | --- |
| 实现者，Linux Python 逻辑/fixture | `python -m unittest discover -s scripts -p test_windows_package.py -v` | 退出 0；35 项，33 pass、2 Windows-only skip |
| 独立审查，Linux Python 逻辑/fixture | `python3 -B -m unittest discover -s scripts -p 'test_windows_package.py' -v` | 退出 0；同一 35 项，33 pass、2 skip；01:25 UTC 复验，不另加总 |
| 实现者，Python 语法 | `python -m py_compile scripts/package_windows.py scripts/test_windows_package.py` | 退出 0 |

覆盖现有 VS2026/2022/Build Tools、多实例选择与回退、缺组件、非正式版本、工具/SDK/CRT 来源、符号链接/重复 DLL、Unicode 路径与遮蔽、CMake 参数/缓存和 Cargo 隔离等。独立源码审查无剩余阻断项。两项跳过是原有真实 Windows CMD/VsDevCmd 路径和 Authenticode 环境用例；不能记为通过。

以上结果对应候选文件 SHA256：

- `scripts/package_windows.py`：`fab85eb0d792b48871e59fb079a553be50c1de32719db5b602c03bb9ba37a7a8`
- `scripts/test_windows_package.py`：`01ced51a74ea8801959b2dba4a108e1e8d25bb9de1f41d5b303919caa2053f56`

## 交付边界与下一步

- 已发送完整 App 仍为 `33f0e17`；独立提供的 `0d5b1dd` 源码/预编译 aria2 组件不是本次修复后的新 App。该组件 ZIP 为 6,669,695 bytes，SHA256 `e81e24001495a62c863d4d439ba0b41a40ede6d007ad658690ec9400d9e7bf3f`，其 Windows 执行仍为 `not_run`
- 本快照记录时，新提交尚未创建，也未重建该新提交的 aria2。形成新提交后由交付方真实重建、复核精确同源组件及对应源码/许可，用户不需要编译 aria2；不可改写旧清单冒充新来源或拼入旧 App
- Rust `1.98.1`、CMake `4.4.3`、固定 llama.cpp 与现有前端锁未变。用户已装 stable MSVC 别名，但本地 `rustc -vV` 的实际 release/host 未验证。后续 pnpm 迁移已明确暂缓，不在此次解阻中改包管理器
- 后续 Rust 构建由用户在本地 Windows 手动执行，不再用 GitHub Actions；用户目前不能连接电脑。本次未执行真实 VsDevCmd/MSVC/CMake、CRT 装载/Authenticode、runtime/桌面 Release 打包、真实模型、下载/取消或目标 Win10 运行，Linux 单测不补足这些证据
- 脚本/文档完成审查后，待新提交及真实同源 aria2 准备好，再给用户匹配输入，完成本地完整包和启动→下载/取消→独立 size/SHA→显式扫描/加载验证；新结果必须记录实际源码、VS/MSVC/SDK 与产物身份
