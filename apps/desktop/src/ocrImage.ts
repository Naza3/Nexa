/** Files are read from an explicit browser input; no filesystem paths cross IPC. */
export async function prepareOcrImage(file: File, edge: number): Promise<string> {
  if (!["image/png", "image/jpeg"].includes(file.type)) throw new Error("仅支持 PNG 或 JPEG 图片。");
  if (file.size > 4 * 1024 * 1024) throw new Error("图片文件不能超过 4 MiB。");
  const url = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error("无法读取图片。"));
    reader.readAsDataURL(file);
  });
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
  if (resized.length > Math.ceil(4 * 1024 * 1024 / 3) * 4 + 32) throw new Error("缩放后图片超过 4 MiB，请选择较小尺寸。");
  return resized;
}
