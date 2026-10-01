/** Explicit DEV preview only. This module is excluded from production builds. */
import { DEFAULT_SETTINGS, wait } from "./controller";
import { DesktopError } from "./adapter";
import type {
  ChatEvent,
  DesktopApi,
  ModelSummary,
  RuntimeStatus,
  Snapshot,
} from "./types";
export function createPreviewApi(): DesktopApi {
  const scenario = new URLSearchParams(window.location.search).get("scenario");
  const runtime: RuntimeStatus = {
    state: "ready",
    selected_model: "qwen3-0.6b-q8",
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
            available: true,
            context_size: 2048,
          },
        ];
  if (scenario === "empty" || scenario === "initial") {
    runtime.state = "unloaded";
    runtime.selected_model = null;
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
      return clone();
    },
    pickModel: async () => ({
      selection_id: "preview-selection",
      file_name: "Qwen3-0.6B-Q8_0.gguf",
      size_bytes: 639898240,
      destination: "%LOCALAPPDATA%\\Nexa\\models（模拟目录）",
    }),
    importModel: async (_, id) => {
      await wait(1600);
      if (models.some((model) => model.id === id))
        throw new DesktopError(
          "model_exists",
          "模型 ID 已存在，请更换 ID 后重新选择文件。",
        );
      const model: ModelSummary = {
        id,
        display_name: id,
        size_bytes: 639898240,
        sha256: "preview-only",
        architecture: "qwen3",
        quantization: "Q8_0",
        validated: true,
        available: true,
        context_size: 2048,
      };
      models = [...models, model];
      return model;
    },
    modelsPage: async () => ({
      data: structuredClone(models),
      next_after: null,
    }),
    loadModel: async (id, options) => {
      runtime.state = "loading";
      await wait(1000);
      runtime.state = "ready";
      runtime.selected_model = id;
      runtime.load_options = options;
      return structuredClone(runtime);
    },
    unloadModel: async () => {
      runtime.state = "unloading";
      await wait(500);
      runtime.state = "unloaded";
      runtime.selected_model = null;
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
