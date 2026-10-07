import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { deferred, makeApi, model, snapshot } from "./fixtures";
import { prepareOcrImage } from "../src/ocrImage";
import type { ChatBatch, DesktopApi, ModelFileSelection, ModelSummary } from "../src/types";
import { DesktopError } from "../src/adapter";
vi.mock("../src/ocrImage", () => ({ prepareOcrImage: vi.fn() }));
beforeEach(() => { vi.mocked(prepareOcrImage).mockResolvedValue("data:image/png;base64,AQ=="); });
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
  fireEvent.click(screen.getByRole("button", { name: "停止识别" }));
  await act(async () => start.resolve({ request_id: "ocr-1" }));
  expect(api.chatCancel).toHaveBeenCalledWith("ocr-1");
  await act(async () => finish.resolve({ request_id: "ocr-1", terminal: true, events: [{ type: "cancelled" }] }));
  expect(screen.getByText(/已停止，已生成内容可能不完整/)).toBeVisible();
});
it("rejects bad images and invalid token limits before any request", async () => {
  vi.mocked(prepareOcrImage).mockRejectedValue(new Error("图片损坏或无法解码。"));
  const api = await mount();
  fireEvent.change(screen.getByLabelText("OCR 图片"), { target: { files: [new File(["bad"], "bad.png", { type: "image/png" })] } });
  expect(await screen.findByText("图片损坏或无法解码。")).toBeVisible();
  expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
  vi.mocked(prepareOcrImage).mockResolvedValue("data:image/png;base64,AQ==");
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
  expect(await screen.findByText(/识别失败：工作进程已结束/)).toBeVisible();
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
