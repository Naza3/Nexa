# ai-runtime CLI

T04 管理二进制，正常依赖只有纯 Rust API、model-store、runtime-core 和 ProcessHost；worker 从当前可执行文件同目录的 ai-runtime-worker（Windows为.exe）启动，不从PATH/CWD搜索。

当前命令：

```
ai-runtime [--data-dir PATH] init
ai-runtime [--data-dir PATH] serve
ai-runtime [--data-dir PATH] models import --id ID --file PATH
ai-runtime [--data-dir PATH] models list
ai-runtime [--data-dir PATH] load ID --backend cpu --context 2048 --threads 2 --batch 128
ai-runtime [--data-dir PATH] unload
ai-runtime [--data-dir PATH] status
ai-runtime [--data-dir PATH] devices
ai-runtime [--data-dir PATH] cancel UUID
ai-runtime [--data-dir PATH] stop
ai-runtime version --json
```

无 start/delete/verify。init 幂等且不轮换现有凭据；serve不创建缺失token、不注册后台服务。当前已验证矩阵的真实smoke显式context2048/threads2/batch128/cpu，配置默认context4096不静默降级。

服务不在运行时，init/import/list使用同一实例锁；open/import/hash在blocking边界执行。运行中import/list及管理命令走经过同连接HMAC服务端proof验证的HTTP接口，Bearer只在验证后发送。列表自动遍历有界分页；实例UUID、PID/进程创建身份与发现文件只是定位信息，不能代替proof。

stop仅在HTTP确认实际core/worker清理且原实例锁/记录释放后成功。历史清理未确认不能因进程消失而冒充停止；失败返回非零。不猜PID、不杀无关进程、不自动重连或跟随重定向。

测试都使用临时目录和凭据。普通fixture测试不表示真实模型推理；真实模型/HTTP/Windows证据见项目验证记录。


T06外部模型目录由同一data root下的model-library.json描述，serve启动只读索引和有界元数据，离线list合并旧managed与external。旧import仍是用户明确请求的受管理复制导入，外部注册不会令它变成隐式移动。API shutdown、Ctrl+C和serve结束都要确认runtime/worker清理后才释放外部源guard；未确认时不宣称停止，沿用失败标记与非零退出。产品每进程只执行一次serve，未知外部cleanup后不在同进程重建catalog。
