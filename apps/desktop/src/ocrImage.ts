const MAX_IMAGE_BYTES = 4 * 1024 * 1024;

function readFile(blob: Blob, mode: "arrayBuffer"): Promise<ArrayBuffer>;
function readFile(blob: Blob, mode: "dataURL"): Promise<string>;
function readFile(blob: Blob, mode: "arrayBuffer" | "dataURL"): Promise<ArrayBuffer | string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    const failed = () => reject(new Error("无法读取图片，请重新选择文件。"));
    reader.onload = () => {
      const result = reader.result;
      if (result === null || (mode === "dataURL") !== (typeof result === "string")) failed();
      else resolve(result);
    };
    reader.onerror = failed;
    reader.onabort = () => reject(new Error("图片读取已中止，请重新选择文件。"));
    try {
      if (mode === "arrayBuffer") reader.readAsArrayBuffer(blob);
      else reader.readAsDataURL(blob);
    } catch {
      failed();
    }
  });
}

function imageType(bytes: Uint8Array): "image/png" | "image/jpeg" {
  const png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  if (png.every((byte, index) => bytes[index] === byte)) return "image/png";
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return "image/jpeg";
  throw new Error("文件内容不是 PNG 或 JPEG 图片，请选择这两种格式的原始图片。");
}

/** Files are read from an explicit browser input; no filesystem paths cross IPC. */
export async function prepareOcrImage(file: File, edge: number): Promise<string> {
  if (file.size === 0) throw new Error("图片文件为空，请重新选择。");
  if (file.size > MAX_IMAGE_BYTES) throw new Error("图片文件不能超过 4 MiB。");
  const buffer = await readFile(file, "arrayBuffer");
  if (!buffer.byteLength) throw new Error("图片文件为空，请重新选择。");
  if (buffer.byteLength > MAX_IMAGE_BYTES) throw new Error("图片文件不能超过 4 MiB。");
  // Windows MIME associations can be absent or incorrect. Detect the bytes,
  // then normalize only the MIME label while preserving the complete payload.
  const type = imageType(new Uint8Array(buffer));
  const url = await readFile(new Blob([buffer], { type }), "dataURL");
  const image = await new Promise<HTMLImageElement>((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("图片损坏或无法解码。"));
    img.src = url;
  });
  if (!image.width || !image.height || image.width > 8192 || image.height > 8192 || image.width * image.height > 16_777_216) throw new Error("图片尺寸最多 8192 × 8192，且不能超过 16 Mi 像素。");
  if (!edge || Math.max(image.width, image.height) <= edge) return url;
  const scale = edge / Math.max(image.width, image.height);
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(image.width * scale));
  canvas.height = Math.max(1, Math.round(image.height * scale));
  const context = canvas.getContext("2d");
  if (!context) throw new Error("无法缩放图片。");
  context.drawImage(image, 0, 0, canvas.width, canvas.height);
  const resized = canvas.toDataURL("image/png");
  const prefix = "data:image/png;base64,";
  if (!resized.startsWith(prefix)) throw new Error("无法生成缩放后的 PNG 图片，请重新选择尺寸。");
  const payload = resized.slice(prefix.length);
  const bytes = payload.length / 4 * 3 - (payload.endsWith("==") ? 2 : payload.endsWith("=") ? 1 : 0);
  if (bytes > MAX_IMAGE_BYTES) throw new Error("缩放后图片超过 4 MiB，请选择较小尺寸。");
  if (!payload || payload.length % 4 !== 0) throw new Error("无法生成缩放后的 PNG 图片，请重新选择尺寸。");
  return resized;
}
