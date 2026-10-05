# ADR0026：发行许可原文无损整合与十文件上限

- 日期：2026-10-05
- 状态：源码已实现；Python、Rust针对性回归与旧包临时语料已验证，新版原生Windows包待验证
- 范围：Windows runtime、桌面、下载组件和独立验收器的许可展示/校验；不改变第三方源码、许可选择或分发权

## 目标与边界

用户要求减少随包零散许可文件，完整解压后的桌面目录至多十份许可相关文件。先前暂缓构建，最新明确要求本批修改完成后统一在GitHub构建；源码验证和新包结果分开记录。范围包括桌面根、嵌套 `runtime/`、`download/` 的文件，不只计算最外层。索引、版权HTML、NOTICE和Microsoft原文均计入；独立验收器不是桌面目录的一部分，单独保持自包含。aria2法定对应源码归档原样保留，不展开或删减归档内源码/许可来凑数字。

不删版权、NOTICE、重复许可或多重许可选项；不以摘要、链接、SPDX标识替代原文。原 Cargo/npm/Microsoft JSON 许可库存自身的字节也保留。所有组件与来源映射仍可追踪，旧文件路径变为原文标识。原有安全hash、路径、普通文件/重解析点、源码、PE依赖、封闭文件集合及Windows句柄保护不放宽。

## 布局

原生Windows当前实际库存：

- 桌面：`THIRD_PARTY_NOTICES.md`、`licenses/index.json`、`licenses/THIRD_PARTY_LICENSES.txt`、`licenses/COPYRIGHT.html`
- runtime：自身同样四份；脱离桌面后仍完整自包含
- download：`licenses/index.json`、`licenses/THIRD_PARTY_LICENSES.txt`，共两份

交叉构建保留既有Microsoft DOCX/PDF和其他已支持的非UTF-8原件，独立保存为 `licenses/ORIGINAL-<原文件名>`，不转码、不另做压缩包。只有存在此类原件时，才将本层 `THIRD_PARTY_NOTICES.md` 的完整原字节及来源映射并入文本，去掉它的独立副本，腾出一个文件位置。既有实际交叉库存每层一份DOCX，仍为4+4+2。未来库存超出预算会明确失败，不遗漏原文或解除上限。

Rust标准库 `COPYRIGHT-library.html` 原字节直接保存为 `COPYRIGHT.html`，不拼接、重写或跨层共享。文本保持原UTF-8字节、CRLF/LF和尾部换行；原文中即使出现分隔符也由字节长度区分。非Microsoft二进制伪装许可仍拒绝。

## 索引与逐字节契约

`licenses/index.json` 的schema为1，format为 `nexa-license-bundle-v1`。顶层仅有 `schema_version`、`format`、`documents`。每个document仅有：

- `original_path`：原路径，按Unicode/ASCII词典序严格递增，唯一且拒绝大小写别名
- `sha256`、`size_bytes`：原始字节hash及长度；整数不能用bool/float替代
- `stored_path`、`offset_bytes`：实际原件文件或文本里的精确字节范围
- `attributions`：原库存记录原样保留，包括已有版本、源码revision、registry integrity等；原JSON库存和Nexa说明使用明确的自身来源映射

JSON要求UTF-8、无BOM、无重复键、无NaN/Infinity。原库存和归属中的数字仅接受`[-2^63, 2^64-1]`范围的整数字面量，拒绝浮点/指数表示、超界整数和`-0`；不允许通过IEEE-754舍入改变字段。布尔与整数按类型严格区分，嵌套字段同样处理。遇到未来超出此范围的库存会明确失败，不能删字段或改原字节以绕过。文本采用固定前言、含原路径/hash/字节长度的固定header、原始字节、固定footer；校验从第一字节按顺序推进，拒绝错误前言、header/footer、缺口、重叠、越界、尾部隐藏数据及无映射文件。独立HTML/非文本原件offset必须为0，存储路径由原路径唯一确定，原始hash/大小仍核验。

## 下载组件合同

aria2源构建artifact继续生成11份独立许可；其构建脚本、source-lock、`build-manifest.json` 和对应源码归档均不修改。`prepare_download` 先验证原artifact的完整文件闭包，再把最终产品所需的11份原文无损整合。

最终下载payload严格为 `nexa-aria2.exe`、`aria2-1.37.0-nexa-corresponding-source.tar.gz`、`build-manifest.json`、索引和文本；加外层manifest/SHA256SUMS共七文件。11条document的原路径、hash、大小必须逐项匹配原 `build.files`，文本offset/框架逐字节闭合。每条恰有一条 `component/version/source` 归属，严格按锁匹配aria2、LLVM runtime或MinGW工具链来源，不接受额外字段。索引限2MiB，文本限32MiB。

生产Rust下载消费者与Python打包校验器采用同一合成fixture；新增原生实现只接受新布局。不重写已交付旧包，不把新许可布局塞入旧二进制。SHA256证明一致性，不等于发布者认证或法律意见。

## 验收器兼容

原生runtime的独立根notice继续有效。交叉runtime没有独立根notice时，验收器必须找到新索引/文本，验证notice的原路径、精确归属、所有文本段的顺序/框架/offset/hash/长度和独立原件的映射/字节，以及许可文件闭包，才接受替代布局；不能简单取消notice必需项。所有原文的归属必须从保留的Cargo/npm/Microsoft原库存重新构造，逐条逐字段比较，保留记录顺序和额外原有字段；拒绝null/空对象、遗漏/新增/篡改字段和重复归属，不能只检查归属数组非空。原库存与嵌套归属JSON也拒绝重复键。

## 验证与发布限制

实际命令、两份旧包语料及未执行项见[验证记录](../verification/2026-10-05-lossless-license-bundles.md)。语料转换只发生于自动清理的临时目录，没有生成新ZIP或新二进制。用户已恢复本批完成后构建授权；统一源码提交后仍须同源重建及原生Windows完整验收，旧包CI结果不转授本次源码。
