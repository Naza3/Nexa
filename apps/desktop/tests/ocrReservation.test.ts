import { expect, it, vi } from "vitest";
import { DesktopController } from "../src/controller";
import { deferred, makeApi, model, snapshot } from "./fixtures";

it("reserves the gap between OCR images against mutations while allowing status reads", async () => {
  const api = makeApi();
  const controller = new DesktopController(api);
  await controller.refresh();
  const token = controller.acquireOcrBatch();
  expect(token).not.toBeNull();
  expect(controller.acquireOcrBatch()).toBeNull();
  await controller.loadModel(model.id);
  await controller.unload();
  await controller.pickModels();
  await controller.scanModels();
  await controller.startDownload("test-model");
  await controller.stop();
  expect(await controller.send("another request")).toBe(false);
  for (const call of [api.loadModel, api.unloadModel, api.pickModels, api.scanModels, api.downloadStart, api.stop, api.chatStart]) expect(call).not.toHaveBeenCalled();
  vi.mocked(api.snapshot).mockClear();
  await controller.checkService();
  expect(api.snapshot).toHaveBeenCalled();
  expect(controller.ownsOcrBatch(token!)).toBe(true);

  controller.releaseOcrBatch(Symbol("wrong owner"));
  expect(controller.getSnapshot().ocr_batch_active).toBe(true);
  controller.releaseOcrBatch(token!);
  const next = controller.acquireOcrBatch()!;
  controller.releaseOcrBatch(token!);
  expect(controller.ownsOcrBatch(next)).toBe(true);
  controller.releaseOcrBatch(next);
  expect(controller.getSnapshot().ocr_batch_active).toBe(false);
});

it("does not claim OCR while another window operation or external generation owns the service", async () => {
  const picked = deferred<null>();
  let current = snapshot();
  const api = makeApi({ snapshot: vi.fn(async () => current), pickModels: vi.fn(() => picked.promise) });
  const controller = new DesktopController(api);
  expect(controller.acquireOcrBatch()).toBeNull();
  await controller.refresh();
  const picking = controller.pickModels();
  expect(controller.acquireOcrBatch()).toBeNull();
  picked.resolve(null);
  await picking;
  current = snapshot();
  current.runtime!.state = "generating";
  current.runtime!.active_request = "external-request";
  await controller.refresh();
  expect(controller.acquireOcrBatch()).toBeNull();
  current = snapshot();
  await controller.refresh();
  const token = controller.acquireOcrBatch();
  expect(token).not.toBeNull();
  controller.releaseOcrBatch(token!);
});

it("allows the close hook to finish a reserved OCR batch without admitting replacement work", async () => {
  const api = makeApi();
  const controller = new DesktopController(api);
  await controller.refresh();
  const token = controller.acquireOcrBatch()!;
  const terminal = deferred<boolean>();
  const remove = controller.registerPreClose(async () => {
    await terminal.promise;
    controller.releaseOcrBatch(token);
    expect(controller.acquireOcrBatch()).toBeNull();
    return true;
  });
  const closing = controller.close();
  expect(controller.getSnapshot().closing).toBe(true);
  expect(api.close).not.toHaveBeenCalled();
  terminal.resolve(true);
  await closing;
  expect(api.close).toHaveBeenCalledTimes(1);
  expect(controller.getSnapshot().ocr_batch_active).toBe(false);
  remove();
});

it("refreshes after a pre-terminal poll rather than using its stale generating snapshot", async () => {
  const api = makeApi();
  const controller = new DesktopController(api);
  await controller.refresh();
  const token = controller.acquireOcrBatch()!;
  const stale = deferred<ReturnType<typeof snapshot>>();
  vi.mocked(api.snapshot).mockReturnValueOnce(stale.promise).mockResolvedValueOnce(snapshot());
  const oldPoll = controller.refresh();
  const fresh = controller.refreshOcrBatch(token);
  const old = snapshot();
  old.runtime!.state = "generating";
  old.runtime!.active_request = "already-finished";
  stale.resolve(old);
  await oldPoll;
  await fresh;
  expect(controller.getSnapshot().snapshot?.runtime?.state).toBe("ready");
  expect(controller.getSnapshot().snapshot?.runtime?.active_request).toBeNull();
  controller.releaseOcrBatch(token);
});
