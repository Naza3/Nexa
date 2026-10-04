# 2026-10-04 选中文件模型登记验证

基线 main `6167d07cb523cc838e6a6fb082e660d56e9d7f79`；新分支 `codex/nexa-add-model`。边界见 [ADR0021](../decisions/0021-selected-file-model-registration.md)。本页记录提交前源码验证，不把后续联合 Windows 包或实机表现预写为通过。

## 已执行

- `cargo test --workspace --all-targets --locked --offline`：40组423通过/0失败/7既有忽略，exit0
- workspace all-targets clippy `-D warnings`、root/Tauri fmt check、`git diff --check`：exit0
- 前端 typecheck、lint、13文件230项测试、production build：exit0
- Python scripts 全部155项：153通过/2 Windows 专用跳过，exit0
- 独立 model-store/desktop-bridge all-targets169项、壳31项、两crate clippy通过；前端定向118项及typecheck通过，均是上面或各自完整回归的子集，不累加总数
- Windows MSVC目标的两crate all-targets与最终Tauri壳交叉检查通过，仅编译检查、不是Windows运行

使用既有Rust1.98.1、Node24.19.0/npm11.9.0与云端Windows交叉工具缓存；未改vendor、包管理器或依赖锁，未运行GitHub Actions。

## 行为覆盖

- 未选中的超大/损坏相邻文件不读payload，不影响定向登记；允许没有默认目录时跨目录添加
- 旧schema1读取不迁移；提交schema2保留旧库；切默认目录只配置，不枚举新目录，既有条目转显式来源
- 原生选择持有受保护来源，600秒租约；重复选择、显式discard、超时和关闭释放；前端只持opaque ID/文件名/大小
- 同名跨目录、external物理对象别名、同来源内容/文件替换、managed精确原路径幂等及不安全models链接拒绝
- 内容拒绝partial与I/O/身份变化/取消/timeout提交前保旧分开；持久性未确认和已提交事实不伪装回滚
- 单选显式选测，已登记与测试失败/暂缓分开；多选不因只剩一个有效文件就擅自自动测试，不抢占已加载模型
- 下载发布后只登记同身份/预期hash的目标文件；不得通过新目录替换绕过原目录对象约束
- UI启动/切页/刷新不触发discover/reconcile/fullscan；目录设置仅保存；手动扫描明确跳过显式来源

## 审查发现与修正

已修复并复查：手扫显式来源降级/丢失风险、旧schema硬链接别名被新去重规则误拒、下载目录对象替换校验、选择过期/丢弃真实释放、可选测试自持work锁导致busy、partial后误将批量当单选、操作保留总数与本次verified混淆、已保存事实在running阶段违反UI契约、managed精确路径去重边界。

最后一次Windows壳检查发现三处错误提示调用了bridge私有构造（E0624）；改为壳已有错误映射后，独立按同命令重跑成功。此前通过不转授该修补，失败和复验记录均保留。clang-cl探测warning为非致命，最终exit0。

## 未验证与交付注意

未执行 Windows 原生文件选择器、真实写删保护、窗口视觉和新多来源真实模型推理。Linux单元/模拟文件测试与Windows交叉编译不能替代这些验收。Windows本机测试记录问题另片修复；校验超时/不自动卸载设置另片处理。

schema2没有自动生成旧索引备份。升级使用前应保留原索引/配置备份；旧版本拒绝v2，不能直接降级使用同数据根。恢复旧备份会丢弃之后新增的登记记录，但本功能从不删除原GGUF。模型数据、API密钥、完整个人路径和生成正文不纳入报告或源码提交。
