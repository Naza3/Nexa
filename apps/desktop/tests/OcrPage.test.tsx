import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { deferred, makeApi, model, snapshot } from "./fixtures";
import * as ocrImage from "../src/ocrImage";
import type { ChatBatch, DesktopApi, ModelFileSelection, ModelSummary } from "../src/types";
import { DesktopError } from "../src/adapter";
beforeEach(() => { vi.spyOn(ocrImage, "prepareOcrImage").mockResolvedValue("data:image/png;base64,AQ=="); });
afterEach(() => vi.unstubAllGlobals());
const pairedFiles: ModelFileSelection = { selection_id: "pair", expires_in_seconds: 600, files: [{ selection_index: 0, file_name: "model.gguf", size_bytes: 100 }, { selection_index: 1, file_name: "mmproj.gguf", size_bytes: 50 }] };
async function mount(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ snapshot: vi.fn(async () => { const value = snapshot(); value.runtime!.load_options!.context_size = 8192; return value; }), ocrStart: vi.fn(async () => ({ request_id: "ocr-1" })), saveOcrMarkdown: vi.fn(async () => ({ saved: true })), modelsPage: vi.fn(async () => ({ generation: "g", data: [{ ...model, has_projector: true }], next_after: null })), ...overrides });
  const controller = new DesktopController(api);
  render(<App controller={controller} initialPage="ocr" />);
  await screen.findByRole("option", { name: /视觉配对/ });
  fireEvent.change(screen.getByLabelText("OCR 模型"), { target: { value: model.id } });
  return api;
}
async function upload() {
  fireEvent.change(screen.getByLabelText("OCR 图片"), { target: { files: [new File(["png"], "page.png", { type: "image/png" })] } });
  await screen.findByAltText("待识别图片预览");
}
it("opens the hidden native image input through the visible labeled button", async () => {
  await mount();
  const input = screen.getByLabelText("OCR 图片");
  const open = vi.spyOn(input, "click").mockImplementation(() => {});
  const button = screen.getByRole("button", { name: "选择图片" });
  expect(input).not.toBeVisible();
  expect(button).toBeVisible();
  expect(button).toHaveAccessibleDescription(/PNG \/ JPEG，最多 4 MiB/);
  fireEvent.click(button);
  expect(open).toHaveBeenCalledTimes(1);
});
it("requires a prepared image even when the selected OCR model is already loaded", async () => {
  const api = await mount();
  expect(screen.getByText("模型已加载，请先选择图片。")).toBeVisible();
  expect(screen.getByRole("status", { name: "OCR 图片准备状态" })).toHaveTextContent("尚未选择图片。");
  expect(screen.getByLabelText("OCR 图片")).toHaveAttribute("accept", ".png,.jpg,.jpeg,image/png,image/jpeg");
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  await upload();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeEnabled();
  fireEvent.change(screen.getByLabelText("OCR 图片"), { target: { files: [] } });
  expect(screen.queryByAltText("待识别图片预览")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  expect(screen.getByText("模型已加载，请先选择图片。")).toBeVisible();
  expect(api.ocrStart).not.toHaveBeenCalled();
});
it("shows preparation and failure beside the preview and retries the same File object", async () => {
  const pending = deferred<string>();
  vi.mocked(ocrImage.prepareOcrImage).mockImplementationOnce(() => pending.promise);
  const api = await mount();
  const file = new File(["png"], "retry.png", { type: "image/png" });
  const input = screen.getByLabelText("OCR 图片");
  fireEvent.change(input, { target: { files: [file] } });
  const feedback = screen.getByRole("status", { name: "OCR 图片准备状态" });
  expect(feedback).toHaveTextContent("已选择：retry.png");
  expect(feedback).toHaveTextContent("正在检查原图");
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  await act(async () => pending.reject(new DesktopError("ocr_image_invalid", "图片损坏或无法解码。")));
  expect(feedback).toHaveTextContent("图片损坏或无法解码。");
  expect(screen.getByText("图片准备失败，请重新选择图片。")).toBeVisible();
  expect(screen.queryByAltText("待识别图片预览")).not.toBeInTheDocument();
  fireEvent.change(input, { target: { files: [file] } });
  await screen.findByAltText("待识别图片预览");
  expect(ocrImage.prepareOcrImage).toHaveBeenCalledTimes(2);
  expect(feedback).toHaveTextContent("原图已准备完成");
  expect(screen.queryByText("图片损坏或无法解码。")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeEnabled();
  expect(input).toHaveValue("");
  expect(api.ocrStart).not.toHaveBeenCalled();
});
it.each(["success", "failure"])("ignores the previous image's late %s after selecting a new image", async (outcome) => {
  const old = deferred<string>();
  vi.mocked(ocrImage.prepareOcrImage).mockImplementationOnce(() => old.promise).mockResolvedValueOnce("data:image/png;base64,TkVX");
  const api = await mount();
  const input = screen.getByLabelText("OCR 图片");
  fireEvent.change(input, { target: { files: [new File(["old"], "old.png")] } });
  fireEvent.change(input, { target: { files: [new File(["new"], "new.png")] } });
  expect(await screen.findByAltText("待识别图片预览")).toHaveAttribute("src", "data:image/png;base64,TkVX");
  await act(async () => { if (outcome === "success") old.resolve("data:image/png;base64,T0xE"); else old.reject(new Error("旧图片解码失败")); });
  expect(screen.getByAltText("待识别图片预览")).toHaveAttribute("src", "data:image/png;base64,TkVX");
  expect(screen.getByRole("status", { name: "OCR 图片准备状态" })).toHaveTextContent("已选择：new.png");
  expect(screen.queryByText("旧图片解码失败")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeEnabled();
  expect(api.ocrStart).not.toHaveBeenCalled();
});
it("waits for the current resize operation and never sends an older preparation", async () => {
  const original = deferred<string>();
  const resized = deferred<string>();
  vi.mocked(ocrImage.prepareOcrImage).mockImplementationOnce(() => original.promise).mockImplementationOnce(() => resized.promise);
  const api = await mount();
  const file = new File(["png"], "resize.png");
  fireEvent.change(screen.getByLabelText("OCR 图片"), { target: { files: [file] } });
  fireEvent.change(screen.getByLabelText("发送图片尺寸"), { target: { value: "1600" } });
  expect(screen.getByRole("status", { name: "OCR 图片准备状态" })).toHaveTextContent("正在检查并准备所选尺寸的图片");
  await act(async () => original.resolve("data:image/png;base64,T0xE"));
  expect(screen.queryByAltText("待识别图片预览")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  expect(api.ocrStart).not.toHaveBeenCalled();
  await act(async () => resized.resolve("data:image/png;base64,TkVX"));
  expect(ocrImage.prepareOcrImage).toHaveBeenNthCalledWith(2, file, 1600);
  expect(screen.getByRole("status", { name: "OCR 图片准备状态" })).toHaveTextContent("预览为本次将发送的图片");
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  await waitFor(() => expect(api.ocrStart).toHaveBeenCalledWith(expect.objectContaining({ image_data_url: "data:image/png;base64,TkVX" })));
});
it("previews an empty-MIME PNG through the real preparation helper and sends normalized bytes", async () => {
  vi.mocked(ocrImage.prepareOcrImage).mockRestore();
  expect(vi.isMockFunction(ocrImage.prepareOcrImage)).toBe(false);
  const decoded: string[] = [];
  class DecodedImage {
    width = 1;
    height = 1;
    onload = () => {};
    set src(value: string) { decoded.push(value); queueMicrotask(() => this.onload()); }
  }
  vi.stubGlobal("Image", DecodedImage);
  const png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=";
  const bytes = Uint8Array.from(atob(png), (value) => value.charCodeAt(0));
  const file = new File([bytes], "扫描页.PNG");
  expect(file.type).toBe("");
  const api = await mount({ chatNext: vi.fn(async (): Promise<ChatBatch> => ({ request_id: "ocr-1", terminal: true, events: [{ type: "cancelled" }] })) });
  fireEvent.change(screen.getByLabelText("OCR 图片"), { target: { files: [file] } });
  const normalized = `data:image/png;base64,${png}`;
  expect(await screen.findByAltText("待识别图片预览")).toHaveAttribute("src", normalized);
  expect(decoded).toEqual([normalized]);
  expect(screen.getByRole("status", { name: "OCR 图片准备状态" })).toHaveTextContent("已选择：扫描页.PNG");
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  await waitFor(() => expect(api.ocrStart).toHaveBeenCalledWith({ model_id: model.id, image_data_url: normalized, prompt: "Text Recognition:", max_output_tokens: 2048 }));
});
it("sends one image and prompt, preserves raw text for copy and native save", async () => {
  const writeText = vi.fn(async () => {});
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  const raw = "# 标题\n\n**正文**";
  const api = await mount({ chatNext: vi.fn(async (): Promise<ChatBatch> => ({ request_id: "ocr-1", terminal: true, events: [{ type: "delta", text: raw }, { type: "completed", finish_reason: "stop", usage: { prompt_tokens: 2, completion_tokens: 2, total_tokens: 4 } }] })) });
  await upload();
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  await screen.findByText("识别完成，请核对原图。");
  expect(api.ocrStart).toHaveBeenCalledWith({ model_id: model.id, image_data_url: "data:image/png;base64,AQ==", prompt: "Text Recognition:", max_output_tokens: 2048 });
  expect(api.chatStart).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "复制识别原文" }));
  expect(writeText).toHaveBeenCalledWith(raw);
  fireEvent.click(screen.getByRole("button", { name: "保存 .md" }));
  expect(api.saveOcrMarkdown).toHaveBeenCalledWith(raw);
  fireEvent.click(screen.getByRole("button", { name: "Markdown" }));
  expect(await screen.findByRole("heading", { name: "标题" })).toBeVisible();
});
it("retains early cancellation until the request id arrives", async () => {
  const start = deferred<{ request_id: string }>();
  const finish = deferred<ChatBatch>();
  const api = await mount({ ocrStart: vi.fn(() => start.promise), chatNext: vi.fn(() => finish.promise) });
  await upload();
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  await waitFor(() => expect(api.ocrStart).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole("button", { name: "停止识别" }));
  await act(async () => start.resolve({ request_id: "ocr-1" }));
  expect(api.chatCancel).toHaveBeenCalledWith("ocr-1");
  await act(async () => finish.resolve({ request_id: "ocr-1", terminal: true, events: [{ type: "cancelled" }] }));
  expect(screen.getByText(/已停止，已生成内容可能不完整/)).toBeVisible();
});
it("rejects bad images and invalid token limits before any request", async () => {
  vi.mocked(ocrImage.prepareOcrImage).mockRejectedValue(new DesktopError("ocr_image_invalid", "图片损坏或无法解码。"));
  const api = await mount();
  fireEvent.change(screen.getByLabelText("OCR 图片"), { target: { files: [new File(["bad"], "bad.png", { type: "image/png" })] } });
  expect(await screen.findByText("图片损坏或无法解码。")).toBeVisible();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  vi.mocked(ocrImage.prepareOcrImage).mockResolvedValue("data:image/png;base64,AQ==");
  await upload();
  fireEvent.change(screen.getByLabelText("最大输出 token"), { target: { value: "4097" } });
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  expect(api.ocrStart).not.toHaveBeenCalled();
});
it("imports the opaque paired selection only after both files and an id", async () => {
  const pending = deferred<ModelSummary>();
  const api = await mount({ pickModelPair: vi.fn(async () => pairedFiles), importModelPair: vi.fn(() => pending.promise) });
  await upload();
  fireEvent.click(screen.getByText("导入 OCR 模型与视觉投影"));
  fireEvent.click(screen.getByRole("button", { name: "选择两个配套文件" }));
  await screen.findByText("主模型：model.gguf");
  expect(screen.getByRole("button", { name: "复制并导入两个文件" })).toBeDisabled();
  fireEvent.change(screen.getByLabelText("模型 ID"), { target: { value: "ocr-model" } });
  fireEvent.click(screen.getByRole("button", { name: "复制并导入两个文件" }));
  await waitFor(() => expect(api.importModelPair).toHaveBeenCalledWith("pair", "ocr-model"));
  expect(screen.getByRole("status", { name: "OCR 模型导入状态" })).toHaveTextContent("正在复制并校验两个模型文件");
  expect(screen.getByRole("button", { name: "正在复制并校验…" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "正在复制并校验…" }));
  expect(api.importModelPair).toHaveBeenCalledTimes(1);
  await act(async () => pending.resolve(model));
  expect(screen.getByRole("status", { name: "OCR 模型导入状态" })).toHaveTextContent("双文件导入完成，源文件保留。");
  expect(screen.queryByLabelText("模型 ID")).not.toBeInTheDocument();
  expect(api.loadModel).not.toHaveBeenCalled();
});
it("shows pending selection beside the picker and blocks duplicate selection and OCR", async () => {
  const pending = deferred<ModelFileSelection | null>();
  const api = await mount({ pickModelPair: vi.fn(() => pending.promise) });
  await upload();
  fireEvent.click(screen.getByText("导入 OCR 模型与视觉投影"));
  fireEvent.click(screen.getByRole("button", { name: "选择两个配套文件" }));
  expect(screen.getByRole("status", { name: "OCR 模型导入状态" })).toHaveTextContent("先选择主模型，再在第二个窗口选择 mmproj");
  expect(screen.getByRole("button", { name: "正在选择配套文件…" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "选择图片" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "正在选择配套文件…" }));
  expect(api.pickModelPair).toHaveBeenCalledTimes(1);
  await act(async () => pending.resolve(pairedFiles));
  expect(screen.getByText("主模型：model.gguf")).toBeVisible();
  expect(screen.getByText("视觉投影：mmproj.gguf")).toBeVisible();
  expect(screen.getByRole("status", { name: "OCR 模型导入状态" })).toHaveTextContent("两个文件已选好");
});
it("refreshes imported managed models even when the service and directory snapshot are unchanged", async () => {
  let imported = false;
  const added = { ...model, id: "glm-ocr-q8", display_name: "GLM OCR", has_projector: true };
  const api = await mount({
    modelsPage: vi.fn(async () => ({ generation: imported ? "after-import" : "before-import", data: [{ ...model, has_projector: true }, ...(imported ? [added] : [])], next_after: null })),
    pickModelPair: vi.fn(async () => pairedFiles),
    importModelPair: vi.fn(async () => { imported = true; return added; }),
  });
  fireEvent.click(screen.getByText("导入 OCR 模型与视觉投影"));
  fireEvent.click(screen.getByRole("button", { name: "选择两个配套文件" }));
  fireEvent.change(await screen.findByLabelText("模型 ID"), { target: { value: added.id } });
  fireEvent.click(screen.getByRole("button", { name: "复制并导入两个文件" }));
  expect(await screen.findByRole("option", { name: "GLM OCR · 视觉配对" })).toBeInTheDocument();
  expect(api.modelsPage).toHaveBeenCalledTimes(2);
  expect(api.loadModel).not.toHaveBeenCalled();
});
it("reports cancellation and drops an old pair instead of silently keeping it", async () => {
  const pick = vi.fn<NonNullable<DesktopApi["pickModelPair"]>>().mockResolvedValueOnce(pairedFiles).mockResolvedValueOnce(null);
  await mount({ pickModelPair: pick });
  fireEvent.click(screen.getByText("导入 OCR 模型与视觉投影"));
  fireEvent.click(screen.getByRole("button", { name: "选择两个配套文件" }));
  await screen.findByText("主模型：model.gguf");
  fireEvent.click(screen.getByRole("button", { name: "选择两个配套文件" }));
  await waitFor(() => expect(screen.getByRole("status", { name: "OCR 模型导入状态" })).toHaveTextContent("已取消文件选择，尚未导入"));
  expect(screen.queryByLabelText("模型 ID")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "选择两个配套文件" })).toBeEnabled();
});
it("keeps the first import confirmation visible when the empty model list becomes populated", async () => {
  let imported = false;
  const added = { ...model, id: "glm-ocr-q8", display_name: "GLM OCR", has_projector: true };
  const api = makeApi({
    modelsPage: vi.fn(async () => ({ generation: imported ? "after-import" : "before-import", data: imported ? [added] : [], next_after: null })),
    pickModelPair: vi.fn(async () => pairedFiles),
    importModelPair: vi.fn(async () => { imported = true; return added; }),
  });
  render(<App controller={new DesktopController(api)} initialPage="ocr" />);
  await waitFor(() => expect(api.modelsPage).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole("button", { name: "选择两个配套文件" }));
  fireEvent.change(await screen.findByLabelText("模型 ID"), { target: { value: added.id } });
  fireEvent.click(screen.getByRole("button", { name: "复制并导入两个文件" }));
  await screen.findByRole("option", { name: "GLM OCR · 视觉配对" });
  expect(screen.getByRole("status", { name: "OCR 模型导入状态" })).toBeVisible();
  expect(screen.getByRole("status", { name: "OCR 模型导入状态" })).toHaveTextContent("双文件导入完成");
  expect(api.loadModel).not.toHaveBeenCalled();
});
it("exposes the native first-file failure beside the picker and allows retry", async () => {
  const pick = vi.fn<NonNullable<DesktopApi["pickModelPair"]>>().mockRejectedValueOnce(new DesktopError("model_directory_unsupported", "请选择本地普通目录中的文件。")).mockResolvedValueOnce(pairedFiles);
  await mount({ pickModelPair: pick });
  fireEvent.click(screen.getByText("导入 OCR 模型与视觉投影"));
  fireEvent.click(screen.getByRole("button", { name: "选择两个配套文件" }));
  await waitFor(() => expect(screen.getByRole("status", { name: "OCR 模型导入状态" })).toHaveTextContent("选择失败：请选择本地普通目录中的文件。（model_directory_unsupported）"));
  expect(screen.queryByLabelText("模型 ID")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "选择两个配套文件" }));
  await screen.findByText("视觉投影：mmproj.gguf");
  expect(pick).toHaveBeenCalledTimes(2);
});
it.each([
  { ...pairedFiles, files: pairedFiles.files.slice(0, 1) },
  { ...pairedFiles, files: [...pairedFiles.files].reverse() },
])("rejects incomplete or reversed native pair results before enabling import", async (selection) => {
  const api = await mount({ pickModelPair: vi.fn(async () => selection), importModelPair: vi.fn() });
  fireEvent.click(screen.getByText("导入 OCR 模型与视觉投影"));
  fireEvent.click(screen.getByRole("button", { name: "选择两个配套文件" }));
  await waitFor(() => expect(screen.getByRole("status", { name: "OCR 模型导入状态" })).toHaveTextContent("response_invalid"));
  expect(screen.queryByLabelText("模型 ID")).not.toBeInTheDocument();
  expect(api.importModelPair).not.toHaveBeenCalled();
});
it("reports consumed import errors without offering reuse of the stale selection", async () => {
  const api = await mount({ pickModelPair: vi.fn(async () => pairedFiles), importModelPair: vi.fn(async () => { throw new DesktopError("selection_expired", "所选项目已过期，请重新选择。"); }) });
  fireEvent.click(screen.getByText("导入 OCR 模型与视觉投影"));
  fireEvent.click(screen.getByRole("button", { name: "选择两个配套文件" }));
  fireEvent.change(await screen.findByLabelText("模型 ID"), { target: { value: "ocr-model" } });
  fireEvent.click(screen.getByRole("button", { name: "复制并导入两个文件" }));
  await waitFor(() => expect(screen.getByRole("status", { name: "OCR 模型导入状态" })).toHaveTextContent("导入未完成：所选项目已过期，请重新选择。（selection_expired）"));
  expect(screen.queryByLabelText("模型 ID")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "选择两个配套文件" })).toBeEnabled();
  expect(api.importModelPair).toHaveBeenCalledTimes(1);
});
it("cancels this window's OCR task when its component unmounts", async () => {
  const finish = deferred<ChatBatch>();
  const api = makeApi({ snapshot: vi.fn(async () => { const value = snapshot(); value.runtime!.load_options!.context_size = 8192; return value; }), ocrStart: vi.fn(async () => ({ request_id: "ocr-owned" })), chatNext: vi.fn(() => finish.promise), modelsPage: vi.fn(async () => ({ generation: "g", data: [{ ...model, has_projector: true }], next_after: null })) });
  const controller = new DesktopController(api);
  const view = render(<App controller={controller} initialPage="ocr" />);
  await screen.findByRole("option", { name: /视觉配对/ });
  fireEvent.change(screen.getByLabelText("OCR 模型"), { target: { value: model.id } });
  await upload();
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  await waitFor(() => expect(api.chatNext).toHaveBeenCalledWith("ocr-owned"));
  view.unmount();
  expect(api.chatCancel).toHaveBeenCalledWith("ocr-owned");
  await act(async () => finish.resolve({ request_id: "ocr-owned", terminal: true, events: [{ type: "cancelled" }] }));
});
it("preserves partial output after a controlled stream failure without replay", async () => {
  const api = await mount({ chatNext: vi.fn(async (): Promise<ChatBatch> => ({ request_id: "ocr-1", terminal: true, events: [{ type: "delta", text: "部分原文" }, { type: "failed", code: "worker_failed", message: "工作进程已结束" }] })) });
  await upload();
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  expect(await screen.findByText(/识别失败：推理进程异常/)).toBeVisible();
  expect(screen.getByText("部分原文")).toBeVisible();
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
});
it("keeps the same OCR stream across navigation and stops it after return", async () => {
  const finish = deferred<ChatBatch>();
  const api = await mount({ chatNext: vi.fn(() => finish.promise) });
  await upload();
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  await waitFor(() => expect(api.chatNext).toHaveBeenCalledWith("ocr-1"));
  fireEvent.click(screen.getByRole("button", { name: "概览" }));
  expect(api.chatCancel).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "图片 OCR" }));
  fireEvent.click(screen.getByRole("button", { name: "停止识别" }));
  expect(api.chatCancel).toHaveBeenCalledWith("ocr-1");
  await act(async () => finish.resolve({ request_id: "ocr-1", terminal: true, events: [{ type: "cancelled" }] }));
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
});
it("loads only on explicit action with visible editable OCR overrides", async () => {
  const api = await mount();
  expect(api.loadModel).not.toHaveBeenCalled();
  expect(screen.getByLabelText("OCR 加载上下文")).toHaveValue(8192);
  fireEvent.change(screen.getByLabelText("OCR 加载线程"), { target: { value: "6" } });
  fireEvent.click(screen.getByRole("button", { name: "加载所选 OCR 模型" }));
  await waitFor(() => expect(api.loadModel).toHaveBeenCalledWith(model.id, { context_size: 8192, batch_size: 256, threads: 6 }));
});
it("does not submit output that alone exhausts the loaded context", async () => {
  const api = await mount({ snapshot: vi.fn(async () => snapshot()) });
  await upload();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  fireEvent.change(screen.getByLabelText("最大输出 token"), { target: { value: "1024" } });
  expect(screen.getByRole("button", { name: "识别图片" })).toBeEnabled();
  expect(api.ocrStart).not.toHaveBeenCalled();
});
it.each(["read", "identity"])("retains ownership through %s failure and failed cancel ACK until terminal is drained", async (failure) => {
  const pending = deferred<ChatBatch>();
  const next = vi.fn<DesktopApi["chatNext"]>();
  if (failure === "read") next.mockRejectedValueOnce(new Error("temporary transport error"));
  else next.mockResolvedValueOnce({ request_id: "wrong-id", terminal: false, events: [] });
  next.mockImplementationOnce(() => pending.promise);
  const api = await mount({ chatNext: next, chatCancel: vi.fn(async () => { throw new Error("cancel ACK lost"); }) });
  await upload();
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  await screen.findByRole("button", { name: "重新确认识别任务" });
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "重新确认识别任务" }));
  await waitFor(() => expect(next).toHaveBeenCalledTimes(2));
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  await act(async () => pending.resolve({ request_id: "ocr-1", terminal: true, events: [{ type: "delta", text: "已生成的部分" }, { type: "cancelled" }] }));
  await waitFor(() => expect(screen.getByRole("button", { name: "识别图片" })).toBeEnabled());
  expect(screen.queryByRole("button", { name: "重新确认识别任务" })).not.toBeInTheDocument();
  expect(screen.getByText("已生成的部分")).toBeVisible();
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
});
it("marks length completion as possibly truncated and preserves copy and save", async () => {
  const api = await mount({ chatNext: vi.fn(async (): Promise<ChatBatch> => ({ request_id: "ocr-1", terminal: true, events: [{ type: "delta", text: "截断文本" }, { type: "completed", finish_reason: "length", usage: { prompt_tokens: 2, completion_tokens: 2048, total_tokens: 2050 } }] })) });
  await upload();
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  expect(await screen.findByText(/已达到输出 token 上限，内容可能截断/)).toBeVisible();
  expect(screen.queryByText("识别完成，请核对原图。")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "复制识别原文" })).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "保存 .md" }));
  expect(api.saveOcrMarkdown).toHaveBeenCalledWith("截断文本");
});
it.each<ChatBatch>([
  { request_id: "ocr-1", terminal: true, events: [] },
  { request_id: "ocr-1", terminal: false, events: [{ type: "cancelled" }] },
  { request_id: "ocr-1", terminal: true, events: [{ type: "cancelled" }, { type: "delta", text: "不得接收" }] },
])("does not release ownership for malformed terminal batches (%j)", async (invalid) => {
  const next = vi.fn<DesktopApi["chatNext"]>().mockResolvedValueOnce(invalid).mockResolvedValueOnce({ request_id: "ocr-1", terminal: true, events: [{ type: "cancelled" }] });
  const api = await mount({ chatNext: next });
  await upload();
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  await screen.findByRole("button", { name: "重新确认识别任务" });
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  expect(screen.queryByText("不得接收")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "重新确认识别任务" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "识别图片" })).toBeEnabled());
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
});
it("keeps incomplete provenance when recovery confirms completed after a lost read", async () => {
  const next = vi.fn<DesktopApi["chatNext"]>().mockRejectedValueOnce(new Error("native batch reply lost")).mockResolvedValueOnce({ request_id: "ocr-1", terminal: true, events: [{ type: "delta", text: "后续片段" }, { type: "completed", finish_reason: "stop", usage: { prompt_tokens: 2, completion_tokens: 8, total_tokens: 10 } }] });
  await mount({ chatNext: next });
  await upload();
  fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  fireEvent.click(await screen.findByRole("button", { name: "重新确认识别任务" }));
  expect(await screen.findByText(/终态已确认，但读取曾中断，原文可能缺失内容/)).toBeVisible();
  expect(screen.queryByText("识别完成，请核对原图。")).not.toBeInTheDocument();
  expect(screen.getByText("后续片段")).toBeVisible();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeEnabled();
});

it("keeps partial OCR output and offers actionable timeout guidance without replay", async () => {
  const api = await mount({ chatNext: vi.fn(async (): Promise<ChatBatch> => ({ request_id: "ocr-1", terminal: true, events: [
    { type: "delta", text: "已识别的部分内容" },
    { type: "failed", code: "execution_timeout", message: "execution timeout" },
  ] })) });
  await upload(); fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
  expect(await screen.findByText(/推理执行超时（execution_timeout）/)).toHaveTextContent("适当调大“推理执行超时”");
  expect(screen.getByText(/推理执行超时（execution_timeout）/)).toHaveTextContent("重新加载模型并手动重试");
  expect(screen.getByText(/推理执行超时（execution_timeout）/)).toHaveTextContent("不会自动重试");
  expect(screen.getByText("已识别的部分内容")).toBeInTheDocument();
  expect(api.ocrStart).toHaveBeenCalledTimes(1); expect(api.chatNext).toHaveBeenCalledTimes(1);
});
