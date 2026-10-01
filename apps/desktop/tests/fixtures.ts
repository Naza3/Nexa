import { vi } from "vitest";
import { DEFAULT_SETTINGS } from "../src/controller";
import type {
  ChatBatch,
  DesktopApi,
  ModelSummary,
  RuntimeStatus,
  Snapshot,
} from "../src/types";
export const model: ModelSummary = {
  id: "qwen",
  display_name: "Qwen 测试模型",
  size_bytes: 1024 * 1024,
  sha256: "a".repeat(64),
  architecture: "qwen3",
  quantization: "Q8_0",
  available: true,
  validated: true,
  context_size: 2048,
};
export function runtime(): RuntimeStatus {
  return {
    state: "ready",
    selected_model: model.id,
    load_options: { context_size: 2048, threads: 2, batch_size: 128 },
    active_request: null,
    queued_jobs: 0,
    stopping: false,
    registry_busy: false,
    configured_backend: "cpu",
    backend: null,
    backend_observation: "unavailable",
    last_error: null,
  };
}
export function snapshot(): Snapshot {
  return {
    initialized: true,
    connection: "connected",
    api_address: "http://127.0.0.1:12345",
    runtime: runtime(),
    settings: { ...DEFAULT_SETTINGS },
  };
}
export function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
export function makeApi(overrides: Partial<DesktopApi> = {}) {
  return {
    snapshot: vi.fn(async () => snapshot()),
    start: vi.fn(async () => snapshot()),
    pickModel: vi.fn(async () => null),
    importModel: vi.fn(async () => model),
    modelsPage: vi.fn(async () => ({ data: [model], next_after: null })),
    loadModel: vi.fn(async () => runtime()),
    unloadModel: vi.fn(async () => ({
      ...runtime(),
      state: "unloaded" as const,
      selected_model: null,
      load_options: null,
    })),
    chatStart: vi.fn(async () => ({ request_id: "request-1" })),
    chatNext: vi.fn(
      async (): Promise<ChatBatch> => ({
        request_id: "request-1",
        events: [
          {
            type: "completed",
            finish_reason: "stop",
            usage: {
              prompt_tokens: 10,
              completion_tokens: 2,
              total_tokens: 12,
            },
          },
        ],
        terminal: true,
      }),
    ),
    chatCancel: vi.fn(async (request_id: string) => ({
      request_id,
      status: "stopping" as const,
    })),
    saveSettings: vi.fn(async () => snapshot()),
    saveIdle: vi.fn(async () => snapshot()),
    copyToken: vi.fn(async () => ({ copied: true as const })),
    stop: vi.fn(async () => ({ stopped: true as const })),
    close: vi.fn(async () => {}),
    ...overrides,
  } satisfies DesktopApi;
}
