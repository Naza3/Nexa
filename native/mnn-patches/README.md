# MNN 私有补丁集

仅针对MNN commit `d407447ed56c4121a11ccbd266dc184ca1ead0c2`，不修改原始共享checkout。

1. `0001-nexa-cpu-request-v1.patch`：owner-only hooks、独立取消检查点、每请求新sampler/显式seed、accepted token hook、无额外末token forward；四个请求源码文件
2. `0002-nexa-private-logging-v1.patch`：全局编译期静默宏、config/tokenizer/unicode直接sink；四个日志源码文件
3. `0003-nexa-compiled-sinks-v1.patch`：实际编译闭包中的core、embedding、omni、dflash、eagle诊断sink；五个额外文件

13个目标的before/after SHA256、补丁顺序/每文件SHA256和patch-set身份全部在`lock.json`。`identity.py`只接受精确干净原版、新建私有副本；应用后核对完整目标集合与postimage，目标文件不允许symlink。已有上游非修改symlink原样保留，不用解引用改变Git身份。

MNN原版源代码/许可由调用者已锁checkout提供（Apache-2.0）；这些patch不打包上游完整代码或模型。构建/测试/限制见[shim README](../mnn-shim/README.md)与[验证记录](../mnn-shim/VERIFICATION.md)。
