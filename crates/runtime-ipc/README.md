# runtime-ipc

T03双方共用的纯Rust私有协议。依赖core/types/serde，不依赖engine-host或原生库。

- `Frame`同时固定版本、session UUID、operation_id、request_id、worker seq和kind/payload
- 父Hello指定session，双方核对实际私有protocol4/shim行为identity4/锁定llama commit；公共protocol仍1，C ABI布局v2不变；旧父worker组合拒绝；无argv生产worker只接受stdin协议
- `encode_frame`在任何pipe写入前验证完整编码；`read_frame`读取前执行上限检查，残缺EOF、未知字段、重复key、非法UTF8都拒绝
- 图片请求8MiB、纯文本Generate仍2MiB、事件64KiB均含LF；TextDelta≤4KiB，编码≤25KiB
- `EventValidator`校验握手、操作、Prepared/usage/终态顺序与一次性信用。应在reader侧验证后才能进入队列；begin/grant与accept共用锁
- `operation_complete`用于避免reader已接受终态而supervisor仍试图补发信用的竞态；消费释放只由core中的permit完成
- 每Generate为16KiB固定暂存+2×120KiB信用，持有到消费者放下lease；不把读取或复制算作消费进展

`cargo test --locked -p runtime-ipc`覆盖编码边界、转义膨胀、版本/身份/序列、重复/未授权信用、重复终态及usage不一致。完整进程行为由process-host/runtime-worker的测试覆盖。

单图以有界 `ImageInput` 跨worker传递，保留图文顺序；Load可带已核验projector路径。普通文本wire省略空image/projector，旧worker仍通过显式版本检查拒绝混搭。
