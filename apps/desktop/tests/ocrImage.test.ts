import { afterEach, expect, it, vi } from "vitest";
import { prepareOcrImage } from "../src/ocrImage";
afterEach(() => vi.unstubAllGlobals());
it("rejects unapproved formats and oversize files before decoding", async () => {
  await expect(prepareOcrImage(new File(["svg"], "x.svg", { type: "image/svg+xml" }), 0)).rejects.toThrow("PNG");
  await expect(prepareOcrImage(new File([new Uint8Array(4 * 1024 * 1024 + 1)], "x.png", { type: "image/png" }), 0)).rejects.toThrow("4 MiB");
});
it("rejects corrupt images and excessive decoded dimensions", async () => {
  class BadImage { onerror = () => {}; set src(_: string) { queueMicrotask(() => this.onerror()); } }
  vi.stubGlobal("Image", BadImage);
  await expect(prepareOcrImage(new File(["bad"], "x.png", { type: "image/png" }), 0)).rejects.toThrow("损坏");
  class HugeImage { width = 8193; height = 1; onload = () => {}; set src(_: string) { queueMicrotask(() => this.onload()); } }
  vi.stubGlobal("Image", HugeImage);
  await expect(prepareOcrImage(new File(["png"], "x.png", { type: "image/png" }), 1600)).rejects.toThrow("8192");
});
it("preserves the original bytes when resizing was not selected", async () => {
  class Image { width = 100; height = 100; onload = () => {}; set src(_: string) { queueMicrotask(() => this.onload()); } }
  vi.stubGlobal("Image", Image);
  await expect(prepareOcrImage(new File(["abc"], "x.png", { type: "image/png" }), 0)).resolves.toBe("data:image/png;base64,YWJj");
});
