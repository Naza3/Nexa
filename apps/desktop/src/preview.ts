/** Explicit DEV preview only. This module is excluded from production builds. */
import { DEFAULT_SETTINGS, wait } from "./controller";
import { DesktopError } from "./adapter";
import type {
  ChatEvent,
  DesktopApi,
  ModelSummary,
  RuntimeStatus,
  Snapshot,
  DirectoryIdentity,
  LibraryOperation,
} from "./types";
export function createPreviewApi(): DesktopApi {
  const scenario = new URLSearchParams(window.location.search).get("scenario");
  const runtime: RuntimeStatus = {
    state: "ready",
    selected_model: "qwen3-0.6b-q8",
    selected_model_display_name: "Qwen3 0.6B",
    load_options: { context_size: 2048, threads: 2, batch_size: 128 },
    active_request: null,
    queued_jobs: 0,
    stopping: false,
    registry_busy: false,
    configured_backend: "cpu",
    backend: null,
    backend_observation: "unavailable",
    last_error: null,
    memory: {
      api_private_bytes: null,
      worker_private_bytes: null,
      gpu_bytes: null,
      observation: "unavailable",
    },
  };
  let snapshot: Snapshot = {
    initialized: scenario !== "initial",
    connection:
      scenario === "initial" || scenario === "stopped"
        ? "stopped"
        : scenario === "error"
          ? "error"
          : "connected",
    api_address: "http://127.0.0.1:18181",
    settings: { ...DEFAULT_SETTINGS },
    model_directory: { configured: null, effective: null, state: "default" },
    runtime,
  };
  let models: ModelSummary[] =
    scenario === "empty" || scenario === "initial"
      ? []
      : [
          {
            id: "qwen3-0.6b-q8",
            display_name: "Qwen3 0.6B",
            size_bytes: 639898240,
            sha256: "预览示例，不代表真实模型校验值",
            architecture: "qwen3",
            quantization: "Q8_0",
            validated: true,
            loadable: true,
            context_limit: 40960,
            compatibility: "admitted",
            available: true,
            context_size: 2048,
            storage: "managed",
            availability_error: null,
          },
        ];
  if (scenario === "empty" || scenario === "initial") {
    runtime.state = "unloaded";
    runtime.selected_model = null;
    runtime.selected_model_display_name = null;
    runtime.load_options = null;
  }
  if (scenario === "faulted") {
    runtime.state = "faulted";
    runtime.last_error = {
      code: "worker_failed",
      message: "模拟：运行进程退出，请重新加载模型。",
    };
  }
  let turn = 0,
    part = 0,
    cancel = false;
  let lastTerminal: ChatEvent | null = null;
  const clone = () => structuredClone(snapshot);
  let catalogGeneration = "00000000-0000-4000-8000-000000000001";
  let libraryId = 0;
  let libraryPolls = 0;
  let libraryCancelled = false;
  let libraryTerminal: LibraryOperation | null = null;
  let pendingDirectory: DirectoryIdentity | null = null;
  const pick = {
    selection_id: "preview-directory-selection",
    display_path: "D:\\本地模型\\Qwen 模型（模拟目录）",
  };
  function startLibrary(apply: boolean) {
    if (snapshot.connection !== "stopped")
      throw new DesktopError("runtime_running", "请先显式停止运行服务。");
    if (!apply && !snapshot.model_directory.configured)
      throw new DesktopError("model_directory_required", "请先选择目录。");
    libraryId++;
    libraryPolls = 0;
    libraryCancelled = false;
    libraryTerminal = null;
    pendingDirectory = apply
      ? {
          directory_id: "00000000-0000-4000-8000-000000000002",
          display_path: pick.display_path,
          library_generation: "00000000-0000-4000-8000-000000000003",
        }
      : snapshot.model_directory.configured;
    return { operation_id: `preview-library-${libraryId}` };
  }
  return {
    snapshot: async () => {
      await wait(60);
      return clone();
    },
    start: async (initialize) => {
      await wait(800);
      if (!snapshot.initialized && !initialize)
        throw new DesktopError("initialization_required", "请先初始化。");
      snapshot.initialized = true;
      snapshot.connection = "connected";
      snapshot.runtime = runtime;
      snapshot.model_directory.effective = snapshot.model_directory.configured;
      snapshot.model_directory.state = snapshot.model_directory.configured
        ? "ready"
        : "default";
      return clone();
    },
    pickDirectory: async () => pick,
    applyDirectory: async () => startLibrary(true),
    scanModels: async () => startLibrary(false),
    libraryNext: async (operation_id) => {
      await wait(70);
      if (libraryTerminal) return libraryTerminal;
      libraryPolls++;
      const base = {
        operation_id,
        examined_entries: 2,
        candidate_files: 1,
        verified_files: libraryPolls > 1 ? 1 : 0,
        result: null,
        error: null,
        failed_file_name: null,
        file_errors: [],
      };
      if (libraryCancelled) {
        libraryTerminal = {
          ...base,
          status: "cancelled",
          phase: "finished",
          terminal: true,
        };
        return libraryTerminal;
      }
      if (libraryPolls === 1)
        return {
          ...base,
          status: "running",
          phase: "verifying",
          terminal: false,
        };
      const identity = pendingDirectory!;
      snapshot.model_directory = {
        configured: identity,
        effective: null,
        state: "stopped",
      };
      catalogGeneration = identity.library_generation;
      const external: ModelSummary = {
        id: "ext-00000000000040008000000000000004",
        display_name: "Qwen3 中文 0.6B Q8_0",
        size_bytes: 639898240,
        sha256: "preview-only",
        architecture: "qwen3",
        quantization: "Q8_0",
        validated: true,
            loadable: true,
            context_limit: 40960,
        compatibility: "admitted",
        available: true,
        context_size: 2048,
        storage: "external",
        availability_error: null,
      };
      models = [
        ...models.filter((model) => model.storage === "managed"),
        external,
      ];
      libraryTerminal = {
        ...base,
        verified_files: 1,
        status: "completed",
        phase: "finished",
        terminal: true,
        result: {
          library_generation: identity.library_generation,
          directory_id: identity.directory_id,
          registered_files: 1,
          available_files: 1,
          rejected_files: 0,
        },
      };
      return libraryTerminal;
    },
    libraryCancel: async (operation_id) => {
      libraryCancelled = true;
      return { operation_id, status: "stopping" };
    },
    modelsPage: async () => ({
      data: structuredClone(models),
      next_after: null,
      generation: catalogGeneration,
    }),
    loadModel: async (id, options) => {
      runtime.state = "loading";
      await wait(1000);
      runtime.state = "ready";
      runtime.selected_model = id;
      runtime.selected_model_display_name =
        models.find((model) => model.id === id)?.display_name ?? null;
      runtime.load_options = options;
      return structuredClone(runtime);
    },
    unloadModel: async () => {
      runtime.state = "unloading";
      await wait(500);
      runtime.state = "unloaded";
      runtime.selected_model = null;
      runtime.selected_model_display_name = null;
      runtime.load_options = null;
      return structuredClone(runtime);
    },
    chatStart: async () => {
      turn += 1;
      part = 0;
      cancel = false;
      lastTerminal = null;
      runtime.state = "generating";
      await wait(250);
      return { request_id: `preview-${turn}` };
    },
    chatNext: async (request_id) => {
      await wait(150);
      let events: ChatEvent[];
      const words = [
        "这是显式开发预览中的模拟回复。\n\n",
        "本地推理是让模型在你的电脑上处理输入，",
        "无需把消息发送到远程模型服务。",
        "\n\n真实桌面运行时，Nexa 会通过原生桥接",
        "连接本机运行服务，并逐批读取模型输出。",
        "\n\n本预览只用于检查界面、停止和清空交互，",
        "不能作为真实模型推理通过的证据。",
      ];
      if (lastTerminal) events = [lastTerminal];
      else if (cancel) {
        lastTerminal = { type: "cancelled" };
        events = [lastTerminal];
      } else if (part === 0) {
        part += 1;
        events = [{ type: "started" }];
      } else if (part <= words.length) {
        events = [{ type: "delta", text: words[part++ - 1] }];
      } else {
        lastTerminal = {
          type: "completed",
          finish_reason: "stop",
          usage: {
            prompt_tokens: 34,
            completion_tokens: 106,
            total_tokens: 140,
          },
        };
        events = [lastTerminal];
      }
      if (lastTerminal) runtime.state = "ready";
      return { request_id, events, terminal: !!lastTerminal };
    },
    chatCancel: async (request_id) => {
      cancel = true;
      return { request_id, status: "stopping" };
    },
    saveSettings: async (settings) => {
      snapshot.settings = { ...snapshot.settings, ...settings };
      return clone();
    },
    saveIdle: async (idle_unload_seconds) => {
      if (snapshot.connection !== "stopped")
        throw new DesktopError("runtime_running", "请先停止运行服务。");
      snapshot.settings.idle_unload_seconds = idle_unload_seconds;
      return clone();
    },
    copyToken: async () => {
      throw new DesktopError(
        "preview_only",
        "浏览器预览不读取真实令牌，也不会写入剪贴板。",
      );
    },
    stop: async () => {
      await wait(600);
      runtime.state = "unloaded";
      runtime.selected_model = null;
      runtime.selected_model_display_name = null;
      runtime.load_options = null;
      snapshot.model_directory = {
        ...snapshot.model_directory,
        effective: null,
        state: snapshot.model_directory.configured ? "stopped" : "default",
      };
      snapshot = {
        ...snapshot,
        connection: "stopped",
        runtime: null,
        api_address: null,
      };
      return { stopped: true };
    },
    close: async () => {
      throw new DesktopError(
        "preview_only",
        "浏览器预览没有原生窗口关闭操作。",
      );
    },
  };
}
