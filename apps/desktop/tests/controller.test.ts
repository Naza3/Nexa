import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import {
  byteLength,
  checkSubmission,
  DesktopController,
  LIMITS,
  validatePreferences,
  DEFAULT_SETTINGS,
} from "../src/controller";
import type { SessionMessage } from "../src/controller";
import type { ChatBatch, ChatEvent } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";
import { preferencesOnly } from "../src/runtimeSettingsValues";
const batch = (events: ChatEvent[], terminal = false): ChatBatch => ({
  request_id: "request-1",
  events,
  terminal,
});
const complete = (): ChatBatch =>
  batch(
    [
      {
        type: "completed",
        finish_reason: "stop",
        usage: { prompt_tokens: 10, completion_tokens: 2, total_tokens: 12 },
      },
    ],
    true,
  );
const cancelled = (): ChatBatch => batch([{ type: "cancelled" }], true);
const message = (content: string, index = 0): SessionMessage => ({
  id: index,
  role: index % 2 ? "assistant" : "user",
  content,
  state: "complete",
});
async function ready(api = makeApi()) {
  const controller = new DesktopController(api);
  await controller.refresh();
  return controller;
}

describe("bounded submission and preferences", () => {
  it("counts UTF-8 bytes, not characters, including serialized request overhead", () => {
    expect(byteLength("中文")).toBe(6);
    expect(() =>
      checkSubmission(
        [],
        "中".repeat(Math.ceil(LIMITS.session / 3)),
        "qwen",
        512,
      ),
    ).toThrow("512 KiB");
    expect(() => checkSubmission([], "\\".repeat(270000), "qwen", 512)).toThrow(
      "发送数据",
    );
    expect(checkSubmission([], "你好", "qwen", 512).messages).toEqual([
      { role: "user", content: "你好" },
    ]);
  });
  it("never silently drops older messages at the 128-message limit", () => {
    const messages = Array.from({ length: 126 }, (_, index) =>
      message("test", index),
    );
    expect(checkSubmission(messages, "next", "qwen", 1).messages).toHaveLength(
      127,
    );
    expect(() =>
      checkSubmission([...messages, message("extra")], "next", "qwen", 1),
    ).toThrow("128");
    expect(messages).toHaveLength(126);
  });
  it("requires explicit removal of incomplete messages before reuse", () => {
    expect(() =>
      checkSubmission(
        [{ ...message("partial"), state: "incomplete" }],
        "next",
        "qwen",
        1,
      ),
    ).toThrow("未完成");
  });
  it("validates integer settings and dependent batch limits", () => {
    expect(validatePreferences(DEFAULT_SETTINGS)).toBeNull();
    expect(validatePreferences({ ...DEFAULT_SETTINGS, threads: 0 })).toContain(
      "线程",
    );
    expect(
      validatePreferences({ ...DEFAULT_SETTINGS, batch_size: 2049 }),
    ).toContain("批次");
    expect(
      validatePreferences({ ...DEFAULT_SETTINGS, context_size: NaN }),
    ).toContain("整数");
  });
});
describe("single streaming owner", () => {
  it("blocks duplicate sends synchronously before chat_start resolves", async () => {
    const start = deferred<{ request_id: string }>();
    const api = makeApi({ chatStart: vi.fn(() => start.promise) });
    const controller = await ready(api);
    const first = controller.send("first");
    expect(await controller.send("duplicate")).toBe(false);
    expect(api.chatStart).toHaveBeenCalledTimes(1);
    start.resolve({ request_id: "request-1" });
    await first;
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("idle"),
    );
  });
  it("preserves cancellation intent before request id arrives", async () => {
    const start = deferred<{ request_id: string }>();
    const api = makeApi({
      chatStart: vi.fn(() => start.promise),
      chatNext: vi.fn(async () => cancelled()),
    });
    const controller = await ready(api);
    const sending = controller.send("test");
    await controller.cancel();
    expect(api.chatCancel).not.toHaveBeenCalled();
    start.resolve({ request_id: "request-1" });
    await sending;
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("idle"),
    );
    expect(api.chatCancel).toHaveBeenCalledExactlyOnceWith("request-1");
    expect(controller.getSnapshot().messages[1].state).toBe("incomplete");
  });
  it("clear waits for actual terminal, not stopping acknowledgement", async () => {
    const terminal = deferred<ChatBatch>();
    const api = makeApi({ chatNext: vi.fn(() => terminal.promise) });
    const controller = await ready(api);
    await controller.send("keep until terminal");
    await controller.clear();
    await controller.clear();
    expect(controller.getSnapshot().messages).toHaveLength(2);
    expect(controller.getSnapshot().clear_pending).toBe(true);
    expect(api.chatCancel).toHaveBeenCalledTimes(1);
    expect(api.stop).not.toHaveBeenCalled();
    terminal.resolve(cancelled());
    await waitFor(() =>
      expect(controller.getSnapshot().messages).toHaveLength(0),
    );
  });
  it("stores one merged reply and the real usage on a legal terminal", async () => {
    const api = makeApi({
      chatNext: vi
        .fn()
        .mockResolvedValueOnce(
          batch([{ type: "started" }, { type: "delta", text: "你好" }]),
        )
        .mockResolvedValueOnce(batch([{ type: "delta", text: "世界" }]))
        .mockResolvedValueOnce(complete()),
    });
    const controller = await ready(api);
    await controller.send("test");
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("idle"),
    );
    expect(controller.getSnapshot().messages[1]).toMatchObject({
      content: "你好世界",
      state: "complete",
      usage: { prompt_tokens: 10, completion_tokens: 2 },
    });
    expect(api.chatNext).toHaveBeenCalledTimes(3);
  });
  it("keeps a failed partial reply incomplete, then allows explicit removal", async () => {
    const api = makeApi({
      chatNext: vi
        .fn()
        .mockResolvedValueOnce(batch([{ type: "delta", text: "partial" }]))
        .mockResolvedValueOnce(
          batch(
            [
              {
                type: "failed",
                code: "context_length_exceeded",
                message: "上下文超限",
              },
            ],
            true,
          ),
        ),
    });
    const controller = await ready(api);
    await controller.send("test");
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("idle"),
    );
    expect(controller.getSnapshot().messages[1]).toMatchObject({
      content: "partial",
      state: "incomplete",
    });
    expect(await controller.send("again")).toBe(false);
    controller.removeIncomplete();
    expect(controller.getSnapshot().messages).toHaveLength(0);
  });
  it("does not clear after connection loss; explicit recovery consumes terminal without resending", async () => {
    const api = makeApi({
      chatNext: vi
        .fn()
        .mockRejectedValueOnce(new Error("untrusted raw error"))
        .mockResolvedValueOnce(cancelled()),
    });
    const controller = await ready(api);
    await controller.send("test");
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("recovery"),
    );
    await controller.clear();
    expect(controller.getSnapshot().messages).toHaveLength(2);
    await controller.recover();
    await waitFor(() =>
      expect(controller.getSnapshot().messages).toHaveLength(0),
    );
    expect(api.chatStart).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().error?.message ?? "").not.toContain(
      "untrusted",
    );
  });
  it("rejects terminal mismatch and never invents successful completion", async () => {
    const api = makeApi({ chatNext: vi.fn(async () => batch([], true)) });
    const controller = await ready(api);
    await controller.send("test");
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("recovery"),
    );
    expect(controller.getSnapshot().messages[1].state).toBe("incomplete");
    expect(api.chatCancel).toHaveBeenCalledTimes(1);
  });
  it("enforces 256 KiB reply cap without truncating or accumulating rejected deltas", async () => {
    let pulls = 0;
    const api = makeApi({
      chatNext: vi.fn(async () =>
        ++pulls <= 17
          ? batch([{ type: "delta", text: "x".repeat(16 * 1024) }])
          : cancelled(),
      ),
    });
    const controller = await ready(api);
    await controller.send("test");
    await waitFor(
      () => expect(controller.getSnapshot().chat_phase).toBe("idle"),
      { timeout: 2000 },
    );
    expect(byteLength(controller.getSnapshot().messages[1].content)).toBe(
      LIMITS.reply,
    );
    expect(controller.getSnapshot().messages[1].state).toBe("incomplete");
    expect(api.chatCancel).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().error?.code).toBe("history_limit");
  });
  it("only has one pull in flight and starts batches at <=30Hz", async () => {
    let live = 0,
      max = 0,
      count = 0;
    const times: number[] = [];
    const api = makeApi({
      chatNext: vi.fn(async () => {
        live++;
        max = Math.max(max, live);
        times.push(performance.now());
        await new Promise((resolve) => setTimeout(resolve, 2));
        live--;
        return ++count < 4 ? batch([{ type: "delta", text: "." }]) : complete();
      }),
    });
    const controller = await ready(api);
    await controller.send("test");
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("idle"),
    );
    expect(max).toBe(1);
    expect(times[3] - times[0]).toBeGreaterThanOrEqual(95);
  });
});
describe("controlled model and settings actions", () => {
  it("refreshes models after an import instead of accepting an older in-flight page", async () => {
    const oldPage = { data: [model], next_after: null, generation: "before-import" };
    const stale = deferred<typeof oldPage>();
    const added = { ...model, id: "glm-ocr-q8", has_projector: true };
    const api = makeApi({ modelsPage: vi.fn().mockResolvedValueOnce(oldPage).mockImplementationOnce(() => stale.promise).mockResolvedValueOnce({ data: [model, added], next_after: null, generation: "after-import" }) });
    const controller = await ready(api);
    const oldRead = controller.loadPage(null);
    const refreshed = controller.refreshModels();
    stale.resolve(oldPage);
    await Promise.all([oldRead, refreshed]);
    expect(api.modelsPage).toHaveBeenCalledTimes(3);
    expect(controller.getSnapshot().models.data.map((item) => item.id)).toEqual([model.id, added.id]);
    expect(controller.getSnapshot().models.generation).toBe("after-import");
  });
  it("retains only the current model page", async () => {
    const api = makeApi({
      modelsPage: vi
        .fn()
        .mockResolvedValueOnce({
          data: [model],
          next_after: "qwen",
          generation: "generation-1",
        })
        .mockResolvedValueOnce({
          data: [{ ...model, id: "second" }],
          next_after: null,
          generation: "generation-1",
        }),
    });
    const controller = await ready(api);
    await controller.loadPage("qwen");
    expect(controller.getSnapshot().models.data.map((item) => item.id)).toEqual(
      ["second"],
    );
  });
  it("coalesces concurrent snapshots and applies idle separately", async () => {
    const next = deferred<ReturnType<typeof snapshot>>();
    const stopped = { ...snapshot(), connection: "stopped" as const, runtime: null };
    const api = makeApi({ snapshot: vi.fn(() => next.promise), saveSettings: vi.fn(async () => stopped) });
    const controller = new DesktopController(api);
    const a = controller.refresh(),
      b = controller.refresh();
    expect(api.snapshot).toHaveBeenCalledTimes(1);
    next.resolve(snapshot());
    await Promise.all([a, b]);
    const preferences = preferencesOnly(DEFAULT_SETTINGS);
    await controller.saveSettings(preferences);
    expect(api.saveSettings).toHaveBeenCalledWith(preferences);
    expect(api.saveIdle).not.toHaveBeenCalled();
    await controller.saveIdle(DEFAULT_SETTINGS.idle_unload_seconds);
    expect(api.saveIdle).toHaveBeenCalledWith(300);
  });
});

describe("additional terminal and history safety", () => {
  it("does not promote a disconnected partial reply to complete on a later terminal summary", async () => {
    const api = makeApi({
      chatNext: vi
        .fn()
        .mockResolvedValueOnce(batch([{ type: "delta", text: "partial" }]))
        .mockRejectedValueOnce(new Error("transport lost"))
        .mockResolvedValueOnce(complete()),
    });
    const controller = await ready(api);
    await controller.send("test");
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("recovery"),
    );
    await controller.recover();
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("idle"),
    );
    expect(controller.getSnapshot().messages[1]).toMatchObject({
      content: "partial",
      state: "incomplete",
    });
    expect(await controller.send("again")).toBe(false);
  });
  it("rejects a too-large batch without rendering its text", async () => {
    const api = makeApi({
      chatNext: vi
        .fn()
        .mockResolvedValueOnce(
          batch([{ type: "delta", text: "x".repeat(LIMITS.batch + 1) }]),
        )
        .mockResolvedValueOnce(cancelled()),
    });
    const controller = await ready(api);
    await controller.send("test");
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("idle"),
    );
    expect(controller.getSnapshot().messages[1].content).toBe("");
    expect(controller.getSnapshot().messages[1].state).toBe("incomplete");
    expect(api.chatCancel).toHaveBeenCalledTimes(1);
  });
  it("enforces the 512 KiB conversation bound across completed turns and incoming deltas", async () => {
    const api = makeApi({
      chatNext: vi
        .fn()
        .mockResolvedValueOnce(
          batch([{ type: "delta", text: "x".repeat(LIMITS.batch) }]),
        )
        .mockResolvedValueOnce(
          batch([{ type: "delta", text: "x".repeat(LIMITS.batch) }]),
        )
        .mockResolvedValueOnce(complete())
        .mockResolvedValueOnce(
          batch([{ type: "delta", text: "y".repeat(2048) }]),
        )
        .mockResolvedValueOnce(cancelled()),
    });
    const controller = await ready(api);
    await controller.send("a");
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("idle"),
    );
    expect(await controller.send("b".repeat(479 * 1024))).toBe(true);
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("idle"),
    );
    expect(controller.getSnapshot().messages).toHaveLength(4);
    expect(controller.getSnapshot().messages[3].state).toBe("incomplete");
    expect(
      controller
        .getSnapshot()
        .messages.reduce(
          (sum, message) => sum + byteLength(message.content),
          0,
        ),
    ).toBeLessThanOrEqual(LIMITS.session);
    expect(api.chatCancel).toHaveBeenCalledTimes(1);
  });
  it("never interprets a failed cancellation acknowledgement as terminal", async () => {
    const next = deferred<ChatBatch>();
    const api = makeApi({
      chatNext: vi.fn(() => next.promise),
      chatCancel: vi.fn().mockRejectedValueOnce({
        code: "cancel_failed",
        message: "停止确认失败",
      }),
    });
    const controller = await ready(api);
    await controller.send("a");
    await controller.clear();
    expect(controller.getSnapshot().messages).toHaveLength(2);
    expect(controller.getSnapshot().chat_phase).toBe("stopping");
    next.resolve(cancelled());
    await waitFor(() =>
      expect(controller.getSnapshot().messages).toHaveLength(0),
    );
  });
});

it("does not let a stale in-flight status overwrite explicitly saved preferences", async () => {
  const pending = deferred<ReturnType<typeof snapshot>>();
  const saved = snapshot();
  saved.settings.threads = 4;
  const api = makeApi({
    snapshot: vi
      .fn()
      .mockResolvedValueOnce(snapshot())
      .mockImplementationOnce(() => pending.promise),
    saveSettings: vi.fn(async () => saved),
  });
  const controller = await ready(api);
  const oldPoll = controller.refresh();
  const preferences = {
    context_size: saved.settings.context_size,
    threads: saved.settings.threads,
    batch_size: saved.settings.batch_size,
    max_output_tokens: saved.settings.max_output_tokens,
    close_runtime_on_exit: saved.settings.close_runtime_on_exit,
    download_source: saved.settings.download_source,
  };
  await controller.saveSettings(preferences);
  pending.resolve(snapshot());
  await oldPoll;
  expect(controller.getSnapshot().snapshot?.settings.threads).toBe(4);
});
