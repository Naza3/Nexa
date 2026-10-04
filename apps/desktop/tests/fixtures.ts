import { vi } from "vitest";
import { DEFAULT_SETTINGS } from "../src/controller";
import type {
  ChatBatch,
  DesktopApi,
  ModelSummary,
  RuntimeStatus,
  Snapshot,
  LibraryOperation,
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
  loadable: true,
  context_limit: 40960,
  compatibility: "admitted",
  context_size: 2048,
  storage: "managed",
  availability_error: null,
};
export function runtime(): RuntimeStatus {
  return {
    state: "ready",
    selected_model: model.id,
    selected_model_display_name: model.display_name,
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
    lan_api: { enabled: false, listen: null, allowed_cidrs: [] },
    initialized: true,
    connection: "connected",
    api_address: "http://127.0.0.1:12345",
    runtime: runtime(),
    settings: { ...DEFAULT_SETTINGS },
    model_directory: { configured: null, effective: null, state: "default" },
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
    catalog: vi.fn(async () => ({ entries: [] })),
    discoverDirectory: vi.fn(async () => null),
    downloadStart: vi.fn(async () => ({ operation_id: "download-1" })),
    downloadNext: vi.fn(async () => ({ operation_id: "download-1", catalog_id: "test-model", source: "modelscope" as const, file_name: "test.gguf", directory_id: "directory-1", target_display_path: "D:\\models", downloaded_bytes: 0, total_bytes: 1024, phase: "finished" as const, status: "cancelled" as const, terminal: true, result: null, error: null })),
    downloadCancel: vi.fn(async () => ({ stopping: true })),
    snapshot: vi.fn(async () => snapshot()),
    start: vi.fn(async () => snapshot()),
    pickDirectory: vi.fn(async () => null),
    pickModels: vi.fn(async () => null),
    discardModelSelection: vi.fn(async () => ({ discarded: true })),
    addModels: vi.fn(async () => ({ operation_id: "library-1" })),
    configureDirectory: vi.fn(async () => ({ operation_id: "library-1" })),
    applyDirectory: vi.fn(async () => ({ operation_id: "library-1" })),
    scanModels: vi.fn(async () => ({ operation_id: "library-1" })),
    reconcileModels: vi.fn(async () => ({ status: "unchanged" as const, operation_id: null })),
    testModel: vi.fn(async () => ({ state: "untested" as const, load_success: false, generation_pass: false, checked_at_unix_ms: null, error_code: null })),
    libraryNext: vi.fn(
      async (): Promise<LibraryOperation> => ({
        operation_id: "library-1",
        status: "completed",
        phase: "finished",
        examined_entries: 1,
        candidate_files: 1,
        verified_files: 1,
        terminal: true,
        result: {
          directory_id: "directory-1",
          library_generation: "generation-2",
          registered_files: 1,
          available_files: 1,
        },
        error: null,
        failed_file_name: null,
      }),
    ),
    libraryCancel: vi.fn(async (operation_id: string) => ({
      operation_id,
      status: "stopping" as const,
    })),
    modelsPage: vi.fn(async () => ({
      data: [model],
      next_after: null,
      generation: "generation-1",
    })),
    loadModel: vi.fn(async () => runtime()),
    unloadModel: vi.fn(async () => ({
      ...runtime(),
      state: "unloaded" as const,
      // The real actor preserves selected_model and load_options after unload.
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
    saveVerificationTimeout: vi.fn(async () => snapshot()),
    copyToken: vi.fn(async () => ({ copied: true as const })),
    lanAddresses: vi.fn(async () => ({ status: "empty" as const, addresses: [] })),
    saveLanSettings: vi.fn(async (lan_api) => ({ ...snapshot(), connection: "stopped" as const, runtime: null, lan_api })),
    copyLanToken: vi.fn(async () => ({ copied: true as const })),
    stop: vi.fn(async () => ({ stopped: true as const })),
    close: vi.fn(async () => {}),
    ...overrides,
  } satisfies DesktopApi;
}
