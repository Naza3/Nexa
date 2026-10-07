import { afterEach, expect, it, vi } from "vitest";
import { prepareOcrImage } from "../src/ocrImage";

// Actual 1 × 1 PNG/JPEG payloads. FileReader and MIME normalization stay real;
// jsdom has no image decoder or canvas, so only those browser APIs are replaced.
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=";
const JPEG = "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAf/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFAEBAAAAAAAAAAAAAAAAAAAAAP/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/AL+AD//Z";
const LIMIT = 4 * 1024 * 1024;

function file(encoded = PNG, type = "image/png", name = "page.png") {
  return new File([Uint8Array.from(atob(encoded), (character) => character.charCodeAt(0))], name, { type });
}
function decoder(width = 1, height = 1, fails = false) {
  const urls: string[] = [];
  class DecodedImage {
    width = width;
    height = height;
    onload = () => {};
    onerror = () => {};
    set src(value: string) {
      urls.push(value);
      queueMicrotask(() => { if (fails) this.onerror(); else this.onload(); });
    }
  }
  vi.stubGlobal("Image", DecodedImage);
  return urls;
}
function canvas(result = `data:image/png;base64,${PNG}`) {
  const drawImage = vi.fn();
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ drawImage } as unknown as CanvasRenderingContext2D);
  const encode = vi.spyOn(HTMLCanvasElement.prototype, "toDataURL").mockReturnValue(result);
  return { drawImage, encode };
}
afterEach(() => vi.unstubAllGlobals());

it.each([
  ["image/png", "", PNG],
  ["image/png", "application/octet-stream", PNG],
  ["image/png", "image/jpeg", PNG],
  ["image/jpeg", "", JPEG],
  ["image/jpeg", "application/octet-stream", JPEG],
  ["image/jpeg", "image/jpg", JPEG],
  ["image/jpeg", "image/png", JPEG],
])("normalizes %s with declared MIME '%s' and preserves actual bytes", async (detected, declared, encoded) => {
  const urls = decoder();
  const expected = `data:${detected};base64,${encoded}`;
  await expect(prepareOcrImage(file(encoded, declared, "renamed.bin"), 0)).resolves.toBe(expected);
  expect(urls).toEqual([expected]);
});

it.each([
  ["<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>", "forged.png", "image/png"],
  ["GIF89a", "forged.jpg", "image/jpeg"],
  ["not an image", "page.png", "image/png"],
  ["\x89PNG\r\n\x1a", "truncated.png", "image/png"],
])("rejects unsupported or truncated signatures despite filename and MIME: %s", async (bytes, name, type) => {
  const urls = decoder();
  await expect(prepareOcrImage(new File([Uint8Array.from(bytes, (character) => character.charCodeAt(0))], name, { type }), 0)).rejects.toThrow("文件内容不是 PNG 或 JPEG");
  expect(urls).toEqual([]);
});

it("rejects empty and oversized files before reading or decoding", async () => {
  const read = vi.spyOn(FileReader.prototype, "readAsArrayBuffer");
  await expect(prepareOcrImage(new File([], "empty.png"), 0)).rejects.toThrow("为空");
  await expect(prepareOcrImage(new File([new Uint8Array(LIMIT + 1)], "large.png"), 0)).rejects.toThrow("4 MiB");
  expect(read).not.toHaveBeenCalled();
});

it("still requires complete browser decoding after an approved signature", async () => {
  decoder(1, 1, true);
  await expect(prepareOcrImage(file(), 0)).rejects.toThrow("损坏或无法解码");
});

it.each([[0, 1], [8193, 1], [1, 8193], [4097, 4096]])("rejects decoded dimensions %i × %i before resizing", async (width, height) => {
  decoder(width, height);
  const { drawImage } = canvas();
  await expect(prepareOcrImage(file(), 1600)).rejects.toThrow("8192");
  expect(drawImage).not.toHaveBeenCalled();
});

it("accepts the exact side and pixel limits without changing the original payload", async () => {
  decoder(8192, 2048);
  await expect(prepareOcrImage(file(), 0)).resolves.toBe(`data:image/png;base64,${PNG}`);
});

it("scales only on explicit request and keeps the aspect ratio", async () => {
  decoder(4000, 3000);
  const { drawImage, encode } = canvas();
  await expect(prepareOcrImage(file(JPEG, ""), 1600)).resolves.toBe(`data:image/png;base64,${PNG}`);
  expect(drawImage).toHaveBeenCalledWith(expect.anything(), 0, 0, 1600, 1200);
  expect(encode).toHaveBeenCalledWith("image/png");
});

it("does not re-encode images that already fit the chosen size", async () => {
  decoder(100, 80);
  const { encode } = canvas();
  await expect(prepareOcrImage(file(JPEG, "image/jpg"), 1600)).resolves.toBe(`data:image/jpeg;base64,${JPEG}`);
  expect(encode).not.toHaveBeenCalled();
});

it.each([LIMIT, LIMIT + 1])("checks the exact resized byte size including base64 padding: %i", async (size) => {
  decoder(4000, 3000);
  const result = `data:image/png;base64,${Buffer.alloc(size).toString("base64")}`;
  canvas(result);
  if (size === LIMIT) await expect(prepareOcrImage(file(), 1600)).resolves.toBe(result);
  else await expect(prepareOcrImage(file(), 1600)).rejects.toThrow("缩放后图片超过 4 MiB");
});

it("rejects a canvas that cannot produce a PNG", async () => {
  decoder(4000, 3000);
  canvas("data:,");
  await expect(prepareOcrImage(file(), 1600)).rejects.toThrow("无法生成缩放后的 PNG");
});

it.each([
  ["readAsArrayBuffer", "error", "无法读取图片"],
  ["readAsArrayBuffer", "abort", "读取已中止"],
  ["readAsDataURL", "error", "无法读取图片"],
  ["readAsDataURL", "abort", "读取已中止"],
] as const)("settles %s %s without leaving preparation pending", async (method, event, message) => {
  const urls = decoder();
  vi.spyOn(FileReader.prototype, method).mockImplementation(function (this: FileReader) {
    queueMicrotask(() => this.dispatchEvent(new ProgressEvent(event)));
  });
  await expect(prepareOcrImage(file(), 0)).rejects.toThrow(message);
  expect(urls).toEqual([]);
});

it("reports synchronous file read failures without exposing OS details", async () => {
  vi.spyOn(FileReader.prototype, "readAsArrayBuffer").mockImplementation(() => { throw new Error("private source path"); });
  await expect(prepareOcrImage(file(), 0)).rejects.toThrow(/^无法读取图片，请重新选择文件。$/);
});
