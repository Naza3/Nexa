# W04 / T0 无模型工具parser探针

日期：2026-10-03。状态：13条行为观察、隔离Release CTest4/4、主代理复验及独立审查通过；发现具体接受判定缺口。本片是证据实验，生产tools/API/协议未实现或变更，没有真实模型、工具执行或DSH结果。后续精确提交4d30bfa的WindowsCI已通过，新增平台证据见末节；它不覆盖后来目录发现/下载工作区。

## 1. 基线、输入与改动范围

- 开始时HEAD为`93dae7e8f7168e631262f36bebc3a44fb1f0d008`、tree`fc499ee571da7f298abcb9cb00f0eacac5077c25`，已发送产品仍43ad5c2，用户目标机未确认运行
- 固定llama.cpp commit `2149c00f4442dc59302e134a02e4c99d5f7ed9fc`，没有升级引擎
- 上游源码模板`models/templates/Qwen-Qwen3-0.6B.jinja`，SHA256`87a2728cb8dc9fe424d624542f6060ec05a1d285ebbec578bb078900e33396b5`。这是源码fixture，不是既有真实GGUF嵌入模板57f1fd…0361，不能互授验证
- 输入[合成工具回合fixture](../../tests/fixtures/tools/single-tool-cycle.json)，SHA256`e9bb4285d526e3d1093a883c7ad2b11ecb31e7411c0860fcf8ab96fb2b743031`；保留`origin:synthetic-design`与未执行真实工具的标记，没有修改fixture或下载权重
- 代码/构建变更仅3文件：新增`native/llama-shim/tests/tool_parser_test.cpp`、CMake新增该测试target，以及Windows工作流既有显式native build列表补`air-tool-parser-test`；生产源码、API、IPC、ABI、包版本不变
- 当前公共API/proof1、私有IPC2、shim行为identity3、C ABI布局v2保持。未来工具生产迁移尚未分配版本，见[工具契约](../windows-tools-contract.md)

3文件冻结范围SHA256为`8ec579d116fa9a1ec2ce169e7c8e59e6d047f6af6a5ceef7076aaf2b726a5d25`，算法为按路径排序后拼接路径UTF-8、NUL、文件内容、NUL。该值仅标识审查范围，不是Git提交、签名或产品manifest。探针源码自身SHA256为`3a5ca15c4bbe5c50adfb8ea8613d22d6a997884ebd8222b55b2db72cfe151cb0`。

## 2. 实际做了什么

使用真实现有Engine初始化日志边界，但不加载模型：先开启common TRACE/Jinja debug，再确认Engine将common阈值降为-1、Jinja debug关闭。随后用固定上游模板与合成messages/tools render第一轮和第二轮历史，检查工具名/schema字段、assistant调用ID与tool结果关联、`tool_response`及合成结果标签保留。没有tokenizer/vocab、采样、生成、真实工具执行或精确token预算测试。

`common_chat_parser_params(applied)`创建后parser仍为空；未显式load时，工具标记被作为普通content返回。调用`params.parser.load(applied.parser)`后才产生当前模板的结构解析。这是空parser回退行为观察，不是生产已实现“空parser拒绝”。

13case分别比较以下三个观测，均不能单独代表接受：

- `strict_complete`：以`COMMON_PEG_PARSE_FLAG_NONE`解析`generation_prompt + raw`，同时要求parse成功且end等于完整输入字节长度
- `final_mapped`/`calls`：执行`common_chat_parse(raw,false,params)`没有抛出异常，以及映射得到的调用数量。锁定上游函数即使`is_partial=false`仍设置LENIENT
- `expected_arguments`：恰好一个调用时，将映射后arguments再解为JSON，与`{"code":"B7"}`比较。它不是原始JSON完整性、重复key、schema或模型正确性的证明

## 3. 13条实际观察

1表示true，0表示false；表中输出均为固定标签、布尔与计数，不记录原始canary内容或异常正文。

| case | strict_complete | final_mapped | calls | expected_arguments | 本次说明 |
| --- | --- | --- | --- | --- | --- |
| valid | 1 | 1 | 1 | 1 | 合成完整调用能解析，尚非生产接受 |
| missing_close_tag | 0 | 1 | 1 | 1 | 缺结束标记仍被final LENIENT映射为调用 |
| truncated_arguments | 0 | 1 | 1 | 0 | 截断参数仍映射为调用，参数比较不匹配 |
| bad_json | 0 | 0 | 0 | 0 | 本次坏JSON反例被拒绝，不推广为所有坏JSON均拒绝 |
| trailing_structure | 0 | 1 | 1 | 1 | 完整调用后追加未结束工具结构仍映射已有调用 |
| unknown_tool | 0 | 0 | 0 | 0 | 本次未知工具名被拒绝 |
| two_calls | 1 | 1 | 2 | 0 | PEG完整匹配允许两调用，不执行首片单调用策略 |
| extra_property | 1 | 1 | 1 | 0 | 完整匹配不能代替additionalProperties检查 |
| duplicate_key | 1 | 1 | 1 | 1 | 再解JSON后的比较可掩盖重复key，不能证明原始参数合规 |
| invalid_enum | 1 | 1 | 1 | 0 | 完整匹配不能代替enum检查 |
| utf8_text | 0 | 1 | 0 | 0 | 普通中英/emoji答复的strict全匹配失败，不能直接用该gate服务auto文本分支 |
| canary_error | 0 | 0 | 0 | 0 | 错误路径未向观察输出/stderr泄露合成canary |
| canary_text | 0 | 1 | 0 | 0 | 正常文本映射路径也未向观察输出/stderr泄露canary |

本次显式`reasoning_format=NONE`的普通文本映射content包含`<think>\n\n</think>\n\n`前缀，再跟原始`Hello, 蓝色 🌈`；`reasoning_content`为空。该generation-prefix现象只记录为当前参数/模板的缺口，不用通用字符串剥离修成生产成功，也不据模型家族名称授予能力。

## 4. 真实检查与失败历史

在Linux隔离Release native目录完成构建及执行；测试target使用`-UNDEBUG`，MSVC配置为`/UNDEBUG`并继承现有UTF-8编译设置，防止Release把assert观察删掉。MSVC配置审查不等于Windows实际执行。

| 检查 | 实际结果 | 范围 |
| --- | --- | --- |
| 隔离Release构建`air-tool-parser-test` | 退出0 | 编译/链接诊断target；没有模型 |
| 直接执行`air-tool-parser-test`并分开捕获stdout/stderr | 退出0；13case断言与观察表一致；stderr 0 bytes | 只输出固定观察信息，canary不在输出中 |
| `ctest --test-dir <隔离native目录> --output-on-failure` | 写入者4/4通过、0.08秒 | stream/template/template-privacy/tool-parser四个测试 |
| 主代理同目录完整CTest复验 | 退出0；4/4通过、0.09秒 | 2026-10-03 10:04 UTC复验，重复运行不计作新增case |
| 独立审查复验四个Release入口 | 全退出0；stderr为0且canary未出现 | 13case断言、3文件构建闭包及生产未改确认，无未解阻断 |

新增CTest以canary正则作为失败条件，并设30秒timeout；直接程序异常仅给固定短错误，不打印parser异常内容。这只证明所执行路径，不是完整工具生命周期、所有日志或Jinja资源安全保证。

初版把普通文本映射能直接形成预想文本结果作为假设，实际运行否定该假设。后续改为明确记录真实generation-prefix与strict失败观察，未通过删掉普通文本反例、改上游、修改生产输出或虚构成功来让测试变绿。对lenient与strict的差异也保留为诊断断言，不把负面行为的断言通过称为工具接受通过。

## 5. 结论与停点

T0已取得可复现证据：final LENIENT映射会容忍本次不完整工具结构；严格PEG全匹配可阻止部分截断，但不能验证JSON重复key/schema/调用数，且当前普通文本分支与generation prefix仍有缺口。因此尚无完整的工具与普通文本接受算法，未证明该模板家族整体工具能力。

本片停在上述具体缺口，生产工具路径保持未开放。下一步只能先明确原始边界/JSON/schema/调用数与普通文本的双分支处理及反例，再单独评估生产纵向实现；不因本测试通过自动升级协议或发送工具事件。本无模型实验没有验证真实工具GGUF、工具token预算/采样停止、官方pi-ai工具wire、DSH或无害工具闭环；后续Windows执行与原固定模型文本回归见末节，用户Win10/i5-8400/16GB仍待验。既有文本/模型/交付结果保持原范围。


## 6. 精确4d30bfa WindowsCI收口

[WindowsCI37115797798](https://github.com/Naza3/Nexa/actions/runs/37115797798)、job`111182258689`最终success，精确源码`4d30bfae815dbdce888f58ec6bf911834dc8dca9`、tree`ba0a30d4f56dfcb86d8fc85cf1cb86ccab2cf1c5`。主代理已下载50份封闭报告并核source/大小/hash，文档写入者再次逐文件复算全部50份一致。

- Windows CTest4/4通过，0.19秒，包含新增air-tool-parser-test及stream/template/template-privacy。该结果证明固定行为观察断言在Windows成立，仍不建立工具接受算法
- 常规Rust48组344 pass/0 fail/7 ignored。既有固定GGUF、HTTP/CLI、包及解压bridge步骤通过；它们是原文本/存储产品回归，不是工具生成或DSH闭环
- 桌面报告`package_unchanged=true`、`native_window_tested=false`，没有用户原生窗口/目标机证据
- 本次没有另行发送T0新二进制，最新用户已发送包仍43ad5c2。随后自动models发现、HF/MS目录下载是未包含于4d30bfa的独立工作区增量，不能继承本CI通过结论

此处只关闭T0精确提交的Windows证据环节，13case暴露的LENIENT/严格完整性/schema/普通文本缺口保持原结论，生产工具路径继续未开放。
