/** Explicit DEV preview only. This module is excluded from production builds. */
import { DEFAULT_SETTINGS, wait } from "./controller";
import { DesktopError } from "./adapter";
import { DEFAULT_LAN_SETTINGS, validateLanSettings } from "./lanApi";
import { preferencesOnly, validateExecutionSeconds, validateIdleSeconds, validateVerificationSeconds } from "./runtimeSettingsValues";
import type {
  ChatEvent,
  DesktopApi,
  ModelSummary,
  ModelLoadOperation,
  LoadOptions,
  RuntimeStatus,
  Snapshot,
  DirectoryIdentity,
  LibraryOperation,
  PerformanceSnapshot,
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
    lan_api: { ...DEFAULT_LAN_SETTINGS },
    initialized: scenario !== "initial",
    connection:
      scenario === "initial" || scenario === "stopped" || scenario === "runtime-settings"
        ? "stopped"
        : scenario === "error"
          ? "error"
          : "connected",
    api_address: "http://127.0.0.1:18181",
    settings: { ...DEFAULT_SETTINGS },
    model_directory: { configured: null, effective: null, state: "default" },
    runtime,
  };
  if (scenario === "runtime-settings") snapshot.configuration = {
    schema_version: 2, revision: `sha256:${"0".repeat(64)}`, saved: {
      global_defaults: { context_size: 2048, threads: 2, batch_size: 128 },
      request_defaults: { max_output_tokens: 768, temperature: 0.8, top_p: 0.95 },
      runtime: { execution_timeout_seconds: 300, idle_unload_enabled: true, idle_unload_seconds: 300, model_verification_timeout_seconds: 300 },
      local_api: { listen: "127.0.0.1:18181" }, lan_api: { ...DEFAULT_LAN_SETTINGS }, model_profiles: [],
    }, runtime_effective: null, pending_restart: false,
    migration: { state: "not_needed", preferences_revision: null, differences: [], backup_available: false },
  };
  let configurationVersion = 0;
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
  let configureOnly = false;
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
    configureOnly = false;
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
  let loadTask: { progress: ModelLoadOperation; options: LoadOptions; step: number; cancel: boolean } | null = null;
  return {
    performanceGet: async () => scenario === "performance" ? structuredClone(PREVIEW_PERFORMANCE) : { instance_id: "preview-empty", capacity: 200, records: [] },
    loadModelStart: async (operation_id, model_id, options) => {
      if (loadTask && !loadTask.progress.terminal) throw new DesktopError("runtime_busy", "预览加载仍在进行。");
      loadTask = { progress: { operation_id, model_id, phase: "preparing", status: "running", terminal: false,
        runtime: structuredClone(runtime), local_validation: null, error: null }, options, step: 0, cancel: false };
      return { operation_id };
    },
    modelLoadNext: async (operation_id) => {
      const task = loadTask;
      if (!task || task.progress.operation_id !== operation_id) throw new DesktopError("request_not_owned", "不是当前预览任务。");
      if (task.progress.terminal) return structuredClone(task.progress);
      await wait(200);
      if (task.cancel) {
        if (runtime.state === "loading") runtime.state = "unloaded";
        task.progress = { ...task.progress, phase: "finished", status: "cancelled", terminal: true,
          runtime: structuredClone(runtime), error: { code: "request_cancelled", message: "预览任务已取消" } };
      } else {
        task.step++;
        if (task.step === 2) { runtime.state = "loading"; task.progress.phase = "loading"; }
        if (task.step === 3) {
          runtime.state = "ready"; runtime.selected_model = task.progress.model_id;
          runtime.selected_model_display_name = models.find((model) => model.id === task.progress.model_id)?.display_name ?? null;
          runtime.load_options = task.options; task.progress.phase = "testing";
        }
        if (task.step >= 4) { task.progress.phase = "finished"; task.progress.status = "completed"; task.progress.terminal = true; }
        task.progress.runtime = structuredClone(runtime);
      }
      return structuredClone(task.progress);
    },
    modelLoadCancel: async (operation_id) => {
      if (!loadTask || loadTask.progress.operation_id !== operation_id) throw new DesktopError("request_not_owned", "不是当前预览任务。");
      if (loadTask.progress.terminal) return { stopping: false };
      loadTask.cancel = true; loadTask.progress.status = "cancelling";
      return { stopping: true };
    },
    catalog: async () => ({ entries: [] }),
    discoverDirectory: async () => null,
    downloadStart: async () => { throw new DesktopError("preview_only", "开发预览不下载真实模型。"); },
    downloadNext: async () => { throw new DesktopError("preview_only", "开发预览没有真实下载进度。"); },
    downloadCancel: async () => ({ stopping: true }),
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
      if (snapshot.configuration) {
        snapshot.configuration.runtime_effective = { chat_response_timeout_seconds: 120 + 300 + 30 + snapshot.configuration.saved.runtime.model_verification_timeout_seconds + snapshot.configuration.saved.runtime.execution_timeout_seconds, revision: snapshot.configuration.revision, values: structuredClone(snapshot.configuration.saved) };
        snapshot.configuration.pending_restart = false;
      }
      runtime.lan_api = { enabled: !!snapshot.lan_api?.enabled, listen: snapshot.lan_api?.listen ?? null, running: !!snapshot.lan_api?.enabled };
      snapshot.runtime = runtime;
      snapshot.model_directory.effective = snapshot.model_directory.configured;
      snapshot.model_directory.state = snapshot.model_directory.configured
        ? "ready"
        : "default";
      return clone();
    },
    pickDirectory: async () => pick,
    pickModels: async () => { throw new DesktopError("preview_only", "请在原生桌面中选择 GGUF 文件，浏览器预览不读取真实文件。"); },
    discardModelSelection: async () => ({ discarded: true }),
    addModels: async () => { throw new DesktopError("preview_only", "开发预览不登记真实模型。"); },
    configureDirectory: async () => { const handle = startLibrary(true); configureOnly = true; return handle; },
    applyDirectory: async () => startLibrary(true),
    scanModels: async () => startLibrary(false),
    reconcileModels: async () => ({ status: "unchanged", operation_id: null }),
    testModel: async () => { throw new DesktopError("preview_only", "开发预览不能产生本机模型测试证据。"); },
    libraryNext: async (operation_id) => {
      await wait(70);
      if (libraryTerminal) return libraryTerminal;
      if (configureOnly) {
        const identity = pendingDirectory!;
        if (!libraryCancelled) {
          snapshot.model_directory = { configured: identity, effective: null, state: "stopped" };
          catalogGeneration = identity.library_generation;
        }
        libraryTerminal = { operation_id, examined_entries: 0, candidate_files: 0, verified_files: 0, failed_file_name: null, error: null, terminal: true, phase: "finished", status: libraryCancelled ? "cancelled" : "completed", result: libraryCancelled ? null : { library_generation: identity.library_generation, directory_id: identity.directory_id, registered_files: models.length, available_files: models.length, rejected_files: 0 } };
        return libraryTerminal;
      }
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
      source: snapshot.connection === "connected" ? "runtime" : "local",
      data: structuredClone(models),
      next_after: null,
      generation: catalogGeneration,
    }),
    unregisterModel: async (model_id, generation) => {
      if (loadTask && !loadTask.progress.terminal || snapshot.connection === "connected" && (runtime.active_request || runtime.queued_jobs || runtime.registry_busy || runtime.stopping || ["loading", "generating", "unloading"].includes(runtime.state)))
        throw new DesktopError("runtime_busy", "预览任务仍在进行。");
      if (snapshot.connection === "connected" && runtime.selected_model === model_id && runtime.state !== "unloaded")
        throw new DesktopError("model_unregister_loaded", "请先卸载当前模型。");
      if (generation !== catalogGeneration) throw new DesktopError("model_list_changed", "列表已经变化。");
      if (!models.some((model) => model.id === model_id)) throw new DesktopError("model_not_found", "模型已不在列表中。");
      models = models.filter((model) => model.id !== model_id);
      catalogGeneration = crypto.randomUUID();
      if (runtime.selected_model === model_id) {
        runtime.selected_model = null; runtime.selected_model_display_name = null; runtime.load_options = null;
      }
      return { model_id, removed: true, files_preserved: true };
    },
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
    configurationSave: async (request) => {
      const configuration = snapshot.configuration;
      if (!configuration) throw new DesktopError("configuration_unavailable", "请使用 runtime-settings 预览场景。");
      if (snapshot.connection !== "stopped") throw new DesktopError("runtime_running", "请先停止运行服务。");
      if (request.expected_revision !== configuration.revision) throw new DesktopError("configuration_conflict", "配置版本已变化。");
      if (request.update.kind !== "runtime") throw new DesktopError("preview_only", "此场景仅模拟运行策略保存。");
      const policy = request.update.runtime;
      const validation = validateExecutionSeconds(policy.execution_timeout_seconds) || validateIdleSeconds(policy.idle_unload_seconds) || validateVerificationSeconds(policy.model_verification_timeout_seconds);
      if (validation) throw new DesktopError("invalid_settings", validation);
      configuration.saved.runtime = structuredClone(policy);
      configuration.revision = `sha256:${(++configurationVersion).toString(16).padStart(64, "0")}`;
      return structuredClone(configuration);
    },
    saveSettings: async (settings) => {
      snapshot.settings = { ...snapshot.settings, ...preferencesOnly(settings) };
      return clone();
    },
    saveIdle: async (idle_unload_seconds, idle_unload_enabled) => {
      if (!snapshot.initialized) throw new DesktopError("not_initialized", "请先显式初始化。");
      if (snapshot.connection !== "stopped")
        throw new DesktopError("runtime_running", "请先停止运行服务。");
      const validation = validateIdleSeconds(idle_unload_seconds);
      if (validation) throw new DesktopError("invalid_settings", validation);
      snapshot.settings.idle_unload_seconds = idle_unload_seconds;
      if (idle_unload_enabled !== undefined) snapshot.settings.idle_unload_enabled = idle_unload_enabled;
      return clone();
    },
    saveVerificationTimeout: async (model_verification_timeout_seconds) => {
      if (!snapshot.initialized) throw new DesktopError("not_initialized", "请先显式初始化。");
      if (snapshot.connection !== "stopped") throw new DesktopError("runtime_running", "请先停止运行服务。");
      const validation = validateVerificationSeconds(model_verification_timeout_seconds);
      if (validation) throw new DesktopError("invalid_settings", validation);
      snapshot.settings.model_verification_timeout_seconds = model_verification_timeout_seconds;
      return clone();
    },
    lanAddresses: async () => ({ status: "unsupported", addresses: [] }),
    saveLanSettings: async (lan_api) => {
      if (!snapshot.initialized) throw new DesktopError("not_initialized", "请先显式初始化。");
      if (snapshot.connection !== "stopped") throw new DesktopError("runtime_running", "请先显式停止运行服务。");
      const validation = validateLanSettings(lan_api);
      if (validation) throw new DesktopError("lan_settings_invalid", validation);
      snapshot.lan_api = structuredClone(lan_api);
      return clone();
    },
    copyLanToken: async () => {
      throw new DesktopError("preview_only", "开发预览不生成或复制真实局域网密钥。");
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

/** Synthetic layout fixtures, never measurements or normal preview inference history. */
export const PREVIEW_PERFORMANCE: PerformanceSnapshot = {
  instance_id: "模拟数据-preview-performance", capacity: 200,
  records: Array.from({ length: 6 }, (_, index) => ({
    sequence: 6 - index, request_id: `模拟请求-${6 - index}`, model_id: index % 2 ? "模拟-Qwen3-0.6B" : "模拟-GLM-OCR",
    modality: index % 2 ? "text" : "image", status: index === 4 ? "failed" : index === 5 ? "cancelled" : "completed",
    accepted_at_unix_ms: 1791446400000 - index * 60000, max_output_tokens: 256,
    usage: { prompt_tokens: 1200, completion_tokens: 256 }, timings: { queue_ms: 15, load_ms: 0, execution_ms: 5200 },
    performance: index >= 4 ? null : { timings: { prepare_us: 10000, prefill_us: 2000000, decode_us: 3000000, output_callback_us: 20000 }, load_options: { context_size: 8192, threads: 4, batch_size: 256 } },
    error_code: index === 4 ? "inference_failed" : index === 5 ? "cancelled" : null, finish_reason: index >= 4 ? null : index === 0 ? "length" : "stop",
  })),
};
