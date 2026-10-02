# Android MNN 原生第三方 notice 源文件包

本目录是 CPU 文本 profile 的可复核许可/归属库存，不是完整产品合规保证，也不代表最终 APK 或 `.so` 链接闭包已验证。此轮仅新增本目录，未改变 native 源码、patch、header、lock、构建脚本或 artifact 身份。

## 校验

从仓库根执行：

```sh
python -B native/mnn-shim/notices/verify.py
```

只需 Python 标准库。脚本检查12个必需组件、原文/归属/证据文件的字节数与 SHA256、组件引用和严格文件集合；不访问网络、不编译、不改文件。README、manifest 和校验脚本本身是控制文件，由源码版本管理审查，不能将 manifest 自带 hash 当作可信签名。来源全文 hash用于外部重取复核，校验脚本不声称已经在线验证它。

## 来源与实际使用

- MNN 固定 commit：`d407447ed56c4121a11ccbd266dc184ca1ead0c2`，上游3.6.1
- NDK：`30.0.16248370`；安装清单锁定 Android LLVM commit：`9f872551d3c681d06fd303b36f16ed5c274735eb`
- 已检查实际 Android Ninja依赖、编译源、原生与Rust静态库声明，以及原生/Rust测试ELF符号。最终应用尚未生成，此处不据此声称每个归档成员都会进入最终APK
- `manifest.json`逐项列出源码、固定revision、提取范围、源文件及保存片段的hash；原文以原始字节复制，未翻译或改写
- MNN与Jinja共享MNN根Apache-2.0原文，Jinja独立保留头部归属；并非Python Jinja2
- FlatBuffers两处上游license原文相同，仅保留一份；其2014/2017版权另存
- half保留含Christian Rau版权的完整MIT原文，包括原始换行
- RapidJSON仅保留实际用到的MIT条款和Tencent/Milo Yip版权；未将JSON_checker或Windows msinttypes库存认定为Android依赖
- Skia许可文件的Google 2011版权不替代实际文件的Google 2015与AOSP 2006版权，后者分别原样保存。核心 `source/cv` 在OpenCV扩展开关关闭时仍有编译/链接证据
- TensorFlow库存许可包含2018版权；实际CPU代码的2015/2016版权同时保留
- LLVM compiler-rt、libc++、libc++abi取自NDK随附NOTICE.toolchain的完整组件分段，保留LLVM exception及Legacy条款，不引入该文件无关的工具GPL/OpenMP库存。libunwind原文从安装清单指定的精确Android commit取得
- `libatomic.a`是150字节的纯注释链接器占位文件，无GROUP/INPUT或对象成员，Rust当前manifest和build输出不列它；CMake测试的 `-latomic` 不使占位文件变成独立运行库，atomic实现归compiler-rt

## Unicode证据及界限

上游生成文件没有记录历史Python/UCD版本，历史生成环境仍是 **unknown**。本次分别通过既有Python3.12.14 / unicodedata15.0.0，以及官方UCD15.0.0的UnicodeData + SpecialCasing直接重建。4007个类别范围及1432个小写映射全部匹配；hpp逐字节相同，cpp仅上游末尾多一个空行。该匹配只证明数据可复现，不证明原作者历史Python版本，也不证明15.0.0是唯一可能匹配的版本。

`evidence/unicode-equivalence.json`记录固定生成器及官方输入hash。官方输入：

- https://www.unicode.org/Public/15.0.0/ucd/UnicodeData.txt
- https://www.unicode.org/Public/15.0.0/ucd/SpecialCasing.txt
- https://www.unicode.org/Public/15.0.0/ucd/ReadMe.txt

保留数据文件的2022版权。当前官方 https://www.unicode.org/terms_of_use.html 说明Unicode Data Files/Software默认适用Unicode License v3，除非具体文件另注；15.0.0数据文件指向该terms。随包保存本次官方terms证据和license全文。这是2026-10-02获取的当前官方条款证据，不能写成已查明MNN历史生成或许可环境。Python是复核工具；没有Python运行时代码被加入表或APK的证据。

## 尚待完成的发行门槛

1. Apache许可修改文件的显著变更标记尚未补齐；须后续单独改patch并重锁身份，本轮不动
2. APK必须实际包含notice资产，并提供用户可访问的许可入口；打包完成后核对文件、hash和入口
3. 对最终 `.so`/APK保存链接map并检查实际静态库成员、DT_NEEDED和打包文件，不将当前测试ELF或artifact目录库存当作最终闭包
4. Rust、Flutter、Android应用壳及以后启用的后端/功能依赖需另行审计；模型许可也不由此包覆盖
5. Nexa自身发行许可及其他产品义务另行决定；本目录不提供法律保证
