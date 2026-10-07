# 合成 OCR 输入

`invoice.png` 是开发测试生成的960×300白底黑字图片，没有真实用户数据。三行期望在调用模型前固定，见 `expected.txt`；不得依据模型输出修改期望来获得通过。该文件只验证单图链路，不构成复杂版面准确率数据集。

真实测试需要另行提供已核验的官方 GLM-OCR Q8 主文件和 projector；权重不入源码。生成临时内联输入（仅Python标准库）：

```sh
python - <<'PY'
import base64
from pathlib import Path
image = Path('tests/fixtures/ocr/invoice.png').read_bytes()
Path('/tmp/nexa-invoice.dataurl').write_text('data:image/png;base64,' + base64.b64encode(image).decode('ascii'))
PY
```

在已构建兄弟 `ai-runtime-worker` 的开发环境执行：

```sh
export NEXA_OCR_MODEL=/path/to/GLM-OCR-Q8_0.gguf
export NEXA_OCR_PROJECTOR=/path/to/mmproj-GLM-OCR-Q8_0.gguf
export NEXA_OCR_IMAGE_DATA_URL=/tmp/nexa-invoice.dataurl
cargo test --locked -p runtime-cli --test real_ocr -- --ignored --nocapture
```

Windows 使用自己的临时绝对路径及PowerShell环境变量语法。测试实际启动隔离的服务/worker、配对复制导入、显式加载、SSE识别、控制取消和非流式恢复；成功后正常停服并清理临时托管副本。常规测试默认忽略这项，不把忽略算通过。
