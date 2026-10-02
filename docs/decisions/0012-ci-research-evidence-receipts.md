# ADR0012：同次CI的B2研究测试证据凭据

日期：2026-10-02。状态：已实施并完成恢复后的本地完整链及独立审查；新提交GitHub门禁待验，不是生产模型准入变更。

## 背景与决定

B2研究工厂仅编译于executor的cfg(test)目标。f4fa90的完整CI已通过，fc8/f4的Ubuntu native完整指纹保持相同。合法修改原生源码（包括必要的版权修改标记）会改变精确指纹；每次先失败一轮再抄录新hash不能增加底层覆盖，因此引入同次已完成真实B1链的测试凭据。不能单从当前build_identity自授证据。

执行顺序必须拆为：B1-only rust_linux → research_receipt → b2_linux。固定前置为tools、inputs、patch、baseline、linux_native、native_real、rust_linux七阶段；禁止引用receipt自身或B2结果，避免循环证明。原有Android构建、五个ELF、四项真实B2与完整证据门禁均保持必需。

## 信任与隔离

凭据不是签名、供应链认证或抗恶意执行者机制，只防正常工程中的陈旧、误混、漏门禁和意外回退。可信依据仍为精确受审源码、真实CI执行链和最终GitHub commit/run/outcome/产物独立核验。

生产库不读该环境变量、不解析凭据、不增加feature/运行开关/公开工厂，不向模型manifest写validated。凭据永远为Linux研究，research_only=true、production_admitted=false、android_run=false。Android产品支持仍由设备/引擎/模型/策略矩阵独立审批；本机制不替代ADR0011的设备验证审核。

## 生成门禁

- tools阶段创建随机context；每份前置报告从生成时绑定源码commit/tree、clean、run/attempt/job/context，不能在mint时给旧报告补写身份
- YAML传七步实际outcome，helper另行严格核对schema、成功状态、所有命令exit0、无timeout/log-limit/cleanup-unconfirmed
- 上游精确对照、9组privacy、Rust真实模型、archive/header/source审计全部成立
- 当前manifest完整字节hash、每个archive、header、patch/policy、实际已链接BuildIdentity与报告一致
- candidate-model.json文件hash、store candidate_digest、template hash分别绑定，不能互相冒充
- 七份固定proof及receipt原子封存为只读新bundle，拒绝已有目标/半成品/复用/非固定文件集合；失败不生成可消费凭据

receipt记录schema/purpose、研究标记、context、issued/expires（有效期至多45分钟）、完整subject及七份proof原文hash/outcome。随机context用于防误混，不是认证密钥。

## 仅测试消费

cfg(test)私有模块拒绝重复/未知字段、超限文件、非固定proof名、symlink/越界、错误时间、跨run/source/context、错误target/compiler/模型/模板和任何身份错配。subject必须与实际BuildIdentity、编译入测试的锁、store候选及模板相符。

CI缺/坏凭据必须失败，不静态回退；本地无凭据可使用明确受审Debian完整身份记录。环境变量只选择测试输入，生产没有可达入口。真实测试仍先断言公共composition拒绝未准入模型，然后才进入私有研究组合。

最终stage重新核对receipt/proofs与原阶段报告，B2结果绑定凭据摘要；四项测试不能遗漏或以默认ignored/零测试冒充通过。

## 验收

覆盖任一前置缺失/失败/skipped/漏真实门禁、循环、跨run/source/context、报告/manifest/archive替换、错误身份、重复/额外proof或JSON字段、过期/未来/过长时限、symlink/越界、静态回退及生产仍拒绝。必须实际执行B1→receipt→B2链，不以单元mock替代真实门禁。

改动限Android CI/helpers、executor测试模块及必要dev依赖；根Windows、公共DTO、原生ABI、生产resolver不变。环境变化后恢复的中间稿需重新验证，恢复文本本身不是实现完成证据。
