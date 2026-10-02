import { DesktopError, safeError } from "./adapter";
import type {
  ChatBatch,
  ChatRequest,
  DesktopApi,
  ModelPage,
  DirectorySelection,
  LibraryOperation,
  Preferences,
  RuntimeStatus,
  SafeError,
  Settings,
  Snapshot,
  Usage,
  WireMessage,
} from "./types";

export const LIMITS = {
  messages: 128,
  reply: 256 * 1024,
  session: 512 * 1024,
  request: 512 * 1024,
  batch: 16 * 1024,
} as const;
export const DEFAULT_SETTINGS: Settings = {
  context_size: 2048,
  threads: 2,
  batch_size: 128,
  max_output_tokens: 512,
  idle_unload_seconds: 300,
  close_runtime_on_exit: false,
};
const encoder = new TextEncoder();
export const byteLength = (value: string) => encoder.encode(value).byteLength;
export type MessageState = "complete" | "streaming" | "incomplete";
export interface SessionMessage extends WireMessage {
  id: number;
  state: MessageState;
  usage?: Usage;
  finish_reason?: string;
  notice?: string;
}
export type ChatPhase =
  | "idle"
  | "starting"
  | "streaming"
  | "stopping"
  | "recovery";
export interface ViewState {
  snapshot: Snapshot | null;
  booting: boolean;
  error: SafeError | null;
  notice: string | null;
  operation: string | null;
  models: Omit<ModelPage, "generation"> & { generation: string | null };
  page_after: string | null;
  models_loading: boolean;
  directory_selection: DirectorySelection | null;
  library: LibraryOperation | null;
  library_phase: "idle" | "starting" | "running" | "stopping" | "recovery";
  messages: SessionMessage[];
  chat_phase: ChatPhase;
  clear_pending: boolean;
}
export const wait = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
export function validatePreferences(settings: Preferences): string | null {
  const {
    context_size: context,
    threads,
    batch_size: batch,
    max_output_tokens: output,
  } = settings;
  if (![context, threads, batch, output].every(Number.isSafeInteger))
    return "参数必须是整数。";
  if (context < 32 || context > 131072)
    return "上下文长度须为 32–131072 tokens。";
  if (threads < 1 || threads > 256) return "线程数须为 1–256。";
  if (batch < 1 || batch > Math.min(context, 4096))
    return "批次大小须为 1–min(上下文长度, 4096)。";
  if (output < 1 || output > 4096) return "默认输出预算须为 1–4096 tokens。";
  return null;
}
export function checkSubmission(
  messages: SessionMessage[],
  text: string,
  model_id: string,
  max_output_tokens: number,
): ChatRequest {
  if (messages.some((message) => message.state !== "complete"))
    throw new DesktopError(
      "incomplete_history",
      "存在不完整回复，请先清空会话或移除未完成轮次。",
    );
  if (messages.length + 2 > LIMITS.messages)
    throw new DesktopError(
      "history_limit",
      "会话已达到 128 条消息上限，请显式清空后再发送。",
    );
  const wire: WireMessage[] = [
    ...messages.map(({ role, content }) => ({ role, content })),
    { role: "user", content: text },
  ];
  if (
    wire.reduce((total, message) => total + byteLength(message.content), 0) >=
    LIMITS.session
  )
    throw new DesktopError(
      "history_limit",
      "会话正文达到 512 KiB 上限，请缩短输入或清空会话。",
    );
  const request = { model_id, messages: wire, max_output_tokens };
  if (byteLength(JSON.stringify(request)) > LIMITS.request)
    throw new DesktopError(
      "request_limit",
      "发送数据超过 512 KiB 上限，请缩短输入或清空会话。",
    );
  return request;
}

/** Owns exactly one stream across page changes. No transcript is persisted. */
export class DesktopController {
  private state: ViewState = {
    snapshot: null,
    booting: true,
    error: null,
    notice: null,
    operation: null,
    models: { data: [], next_after: null, generation: null },
    page_after: null,
    models_loading: false,
    directory_selection: null,
    library: null,
    library_phase: "idle",
    messages: [],
    chat_phase: "idle",
    clear_pending: false,
  };
  private listeners = new Set<() => void>();
  private poll: ReturnType<typeof setTimeout> | undefined;
  private mounted = false;
  private pollEpoch = 0;
  private modelsPromise: Promise<void> | null = null;
  private snapshotPromise: Promise<void> | null = null;
  private snapshotEpoch = 0;
  private stream: {
    id: string | null;
    cancel: boolean;
    cancelSent: boolean;
    reading: boolean;
    capped: boolean;
    incompleteReason: string | null;
  } | null = null;
  private libraryTask: {
    id: string | null;
    cancel: boolean;
    cancelSent: boolean;
    reading: boolean;
    lastPull: number;
  } | null = null;
  private modelsEpoch = 0;
  private nextMessage = 0;
  private modelsLoaded = false;
  constructor(readonly api: DesktopApi) {}
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private update(patch: Partial<ViewState>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach((listener) => listener());
  }
  dismissError = () => this.update({ error: null });
  private report(error: unknown) {
    this.update({ error: safeError(error) });
  }
  mount = () => {
    this.mounted = true;
    const epoch = ++this.pollEpoch;
    const tick = async () => {
      if (!this.mounted || epoch !== this.pollEpoch) return;
      await this.refresh();
      if (this.mounted && epoch === this.pollEpoch)
        this.poll = setTimeout(tick, 1000);
    };
    void tick();
    return () => {
      if (epoch !== this.pollEpoch) return;
      this.mounted = false;
      clearTimeout(this.poll);
      void this.cancel();
      void this.cancelLibrary();
    };
  };
  refresh = (): Promise<void> => {
    if (this.snapshotPromise) return this.snapshotPromise;
    const epoch = this.snapshotEpoch;
    this.snapshotPromise = (async () => {
      try {
        const snapshot = await this.api.snapshot();
        if (epoch !== this.snapshotEpoch) return;
        const previous = this.state.snapshot;
        if (
          previous &&
          (previous.connection !== snapshot.connection ||
            JSON.stringify(previous.model_directory) !==
              JSON.stringify(snapshot.model_directory))
        ) {
          ++this.modelsEpoch;
          this.modelsLoaded = false;
          if (["stale", "unsupported"].includes(snapshot.model_directory.state))
            this.update({
              models: { data: [], next_after: null, generation: null },
              page_after: null,
            });
        }
        this.update({ snapshot, booting: false });
        if (
          snapshot.connection === "connected" &&
          !["stale", "unsupported"].includes(snapshot.model_directory.state) &&
          !this.libraryTask &&
          !this.modelsLoaded &&
          !this.state.models_loading
        )
          await this.loadPage(null);
      } catch (error) {
        if (epoch !== this.snapshotEpoch) return;
        this.update({
          booting: false,
          snapshot: this.state.snapshot
            ? { ...this.state.snapshot, connection: "error", runtime: null }
            : null,
        });
        this.report(error);
      } finally {
        this.snapshotPromise = null;
      }
    })();
    return this.snapshotPromise;
  };
  private async action(
    label: string,
    task: () => Promise<void>,
    allowChat = false,
    allowLibrary = false,
  ) {
    if (
      this.state.operation ||
      (!allowChat && this.stream) ||
      (!allowLibrary && this.libraryTask)
    )
      return;
    ++this.snapshotEpoch;
    this.update({ operation: label, error: null, notice: null });
    try {
      await task();
    } catch (error) {
      this.report(error);
    } finally {
      ++this.snapshotEpoch;
      this.update({ operation: null });
    }
  }
  start = (initialize: boolean) =>
    this.action(initialize ? "正在初始化并启动" : "正在启动服务", async () => {
      this.update({ snapshot: await this.api.start(initialize) });
      ++this.modelsEpoch;
      this.modelsLoaded = false;
      if (this.modelsPromise) await this.modelsPromise;
      await this.loadPage(null);
      await this.refresh();
    });
  loadPage = (after: string | null): Promise<void> => {
    if (this.modelsPromise) return this.modelsPromise;
    if (
      this.state.snapshot?.connection !== "connected" ||
      this.libraryTask ||
      ["stale", "unsupported"].includes(
        this.state.snapshot.model_directory.state,
      )
    )
      return Promise.resolve();
    const epoch = this.modelsEpoch;
    this.update({ models_loading: true });
    this.modelsPromise = (async () => {
      let cursor = after;
      let generation = after ? this.state.models.generation : null;
      try {
        for (let attempt = 0; attempt < 2; attempt++) {
          try {
            const models = await this.api.modelsPage(cursor, generation);
            if (epoch !== this.modelsEpoch) return;
            if (models.data.length > 64 || !models.generation)
              throw new DesktopError(
                "invalid_model_page",
                "模型列表缺少有效版本或超过单页上限。",
              );
            this.modelsLoaded = true;
            this.update({ models, page_after: cursor });
            return;
          } catch (error) {
            if (epoch !== this.modelsEpoch) return;
            if (safeError(error).code !== "model_list_changed" || attempt === 1)
              throw error;
            this.modelsLoaded = false;
            this.update({
              models: { data: [], next_after: null, generation: null },
              page_after: null,
              notice: "模型列表已变化，已重新读取第一页。",
            });
            cursor = null;
            generation = null;
          }
        }
      } catch (error) {
        if (epoch === this.modelsEpoch) this.report(error);
      } finally {
        this.modelsPromise = null;
        this.update({ models_loading: false });
      }
    })();
    return this.modelsPromise;
  };
  pickDirectory = () =>
    this.action("正在选择模型目录", async () => {
      const selection = await this.api.pickDirectory();
      if (selection) this.update({ directory_selection: selection });
    });
  discardDirectorySelection = () => {
    if (!this.state.operation && !this.libraryTask)
      this.update({ directory_selection: null });
  };
  applyDirectory = () => this.beginLibrary("apply");
  scanModels = () => this.beginLibrary("scan");
  private async beginLibrary(kind: "apply" | "scan") {
    if (this.libraryTask || this.state.operation || this.stream) return;
    if (this.state.snapshot?.connection !== "stopped") {
      this.report(
        new DesktopError(
          "runtime_running",
          "请先显式停止运行服务，再应用目录或重新扫描。仅卸载模型不够。",
        ),
      );
      return;
    }
    const selection = this.state.directory_selection;
    if (kind === "apply" && !selection) return;
    if (kind === "scan" && !this.state.snapshot.model_directory.configured) {
      this.report(
        new DesktopError(
          "model_directory_required",
          "请先选择并应用模型目录。",
        ),
      );
      return;
    }
    const task = {
      id: null as string | null,
      cancel: false,
      cancelSent: false,
      reading: false,
      lastPull: -Infinity,
    };
    this.libraryTask = task;
    ++this.snapshotEpoch;
    ++this.modelsEpoch;
    this.update({
      library: null,
      library_phase: "starting",
      error: null,
      notice: null,
    });
    try {
      const handle =
        kind === "apply"
          ? await this.api.applyDirectory(selection!.selection_id)
          : await this.api.scanModels();
      if (!handle.operation_id)
        throw new DesktopError(
          "invalid_library_operation",
          "模型库操作未返回有效标识。",
        );
      task.id = handle.operation_id;
      if (kind === "apply") this.update({ directory_selection: null });
      this.update({ library_phase: task.cancel ? "stopping" : "running" });
      if (task.cancel) void this.sendLibraryCancel();
      void this.consumeLibrary();
    } catch (error) {
      this.libraryTask = null;
      ++this.snapshotEpoch;
      this.update({ library_phase: "idle" });
      this.report(error);
    }
  }
  cancelLibrary = async () => {
    if (!this.libraryTask) return;
    this.libraryTask.cancel = true;
    if (this.state.library_phase !== "recovery")
      this.update({ library_phase: "stopping" });
    await this.sendLibraryCancel();
  };
  private async sendLibraryCancel() {
    const task = this.libraryTask;
    if (!task?.id || task.cancelSent) return;
    task.cancelSent = true;
    try {
      await this.api.libraryCancel(task.id);
    } catch (error) {
      if (this.libraryTask === task) {
        task.cancelSent = false;
        this.report(error);
      }
    }
  }
  recoverLibrary = async () => {
    if (!this.libraryTask?.id || this.libraryTask.reading) return;
    this.update({ library_phase: "stopping", error: null });
    this.libraryTask.cancel = true;
    await this.sendLibraryCancel();
    void this.consumeLibrary();
  };
  private async consumeLibrary() {
    const task = this.libraryTask;
    if (!task?.id || task.reading) return;
    task.reading = true;
    try {
      while (this.libraryTask === task) {
        await wait(
          Math.max(0, Math.ceil(1000 - (performance.now() - task.lastPull))),
        );
        if (this.libraryTask !== task) return;
        if (performance.now() - task.lastPull < 1000) continue;
        task.lastPull = performance.now();
        const progress = await this.api.libraryNext(task.id);
        if (this.libraryTask !== task) return;
        const terminal = progress.status !== "running";
        if (
          progress.operation_id !== task.id ||
          progress.terminal !== terminal ||
          (terminal && progress.phase !== "finished") ||
          (progress.status === "completed" &&
            (!progress.result || progress.error)) ||
          (progress.status === "failed" && !progress.error)
        )
          throw new DesktopError(
            "invalid_library_operation",
            "模型库终态数据不完整，尚未确认操作结束。",
          );
        this.update({ library: progress });
        if (!terminal) continue;
        this.libraryTask = null;
        ++this.snapshotEpoch;
        const durabilityUnconfirmed =
          progress.status === "failed" &&
          progress.error?.code === "settings_durability_unconfirmed";
        if (durabilityUnconfirmed) {
          // Rename may already have committed a different generation. Old pages are no longer authoritative.
          ++this.modelsEpoch;
          this.modelsLoaded = false;
          this.update({
            models: { data: [], next_after: null, generation: null },
            page_after: null,
          });
        }
        if (progress.status === "completed") {
          ++this.modelsEpoch;
          this.modelsLoaded = false;
          this.update({
            models: { data: [], next_after: null, generation: null },
            page_after: null,
            notice: `模型目录已保存，登记 ${progress.result!.registered_files} 个文件，其中 ${progress.result!.available_files} 个符合本版本精确矩阵准入。请启动运行服务读取实际可用性。`,
          });
        } else if (progress.status === "cancelled")
          this.update({ notice: "模型库操作已取消，原目录与索引保持不变。" });
        else this.report(progress.error);
        this.update({
          library_phase: "idle",
          operation: "正在重新读取模型目录",
        });
        // Do not coalesce the authoritative post-terminal read with a pre-commit poll.
        // In particular, rename can commit even when durability confirmation fails.
        try {
          if (this.snapshotPromise) await this.snapshotPromise;
          await this.refresh();
        } finally {
          this.update({ operation: null });
        }
        return;
      }
    } catch (error) {
      if (this.libraryTask === task) {
        this.update({ library_phase: "recovery" });
        this.report(error);
        task.cancel = true;
        void this.sendLibraryCancel();
      }
    } finally {
      task.reading = false;
    }
  }
  private setRuntime(runtime: RuntimeStatus) {
    if (this.state.snapshot)
      this.update({ snapshot: { ...this.state.snapshot, runtime } });
  }
  loadModel = (modelId: string) =>
    this.action("正在加载模型", async () => {
      const settings = this.state.snapshot?.settings ?? DEFAULT_SETTINGS;
      this.setRuntime(
        await this.api.loadModel(modelId, {
          context_size: settings.context_size,
          threads: settings.threads,
          batch_size: settings.batch_size,
        }),
      );
      this.update({ notice: "模型已加载，可以开始聊天。" });
    });
  unload = () =>
    this.action("正在卸载模型", async () => {
      this.setRuntime(await this.api.unloadModel());
    });
  saveSettings = (settings: Preferences) =>
    this.action(
      "正在保存偏好",
      async () => {
        const validation = validatePreferences(settings);
        if (validation) throw new DesktopError("invalid_settings", validation);
        this.update({
          snapshot: await this.api.saveSettings(settings),
          notice:
            "偏好已保存。加载参数在下次加载时生效，输出预算用于下次发送。",
        });
      },
      true,
    );
  saveIdle = (seconds: number) =>
    this.action("正在应用空闲卸载设置", async () => {
      if (!Number.isSafeInteger(seconds) || seconds < 1 || seconds > 86400)
        throw new DesktopError(
          "invalid_settings",
          "空闲卸载时间须为 1–86400 秒。",
        );
      this.update({
        snapshot: await this.api.saveIdle(seconds),
        notice: "空闲卸载设置已保存，下次启动运行服务时生效。",
      });
    });
  copyToken = () =>
    this.action(
      "正在复制令牌",
      async () => {
        await this.api.copyToken();
        this.update({
          notice:
            "令牌已复制到系统剪贴板。使用后请及时清除剪贴板，避免与他人分享。",
        });
      },
      true,
    );
  stop = () =>
    this.action("正在停止运行服务", async () => {
      await this.api.stop();
      ++this.modelsEpoch;
      this.modelsLoaded = false;
      this.update({
        snapshot: this.state.snapshot
          ? {
              ...this.state.snapshot,
              connection: "stopped",
              runtime: null,
              api_address: null,
            }
          : null,
        notice: "运行服务已停止。",
      });
      await this.refresh();
    });
  close = () =>
    this.action(
      "正在关闭应用",
      async () => {
        await this.api.close();
      },
      true,
      true,
    );
  send = async (text: string): Promise<boolean> => {
    if (this.stream || this.libraryTask || this.state.operation || !text.trim())
      return false;
    const snapshot = this.state.snapshot;
    if (
      snapshot?.connection !== "connected" ||
      ["stale", "unsupported"].includes(snapshot.model_directory.state) ||
      snapshot.runtime?.state !== "ready" ||
      snapshot.runtime.stopping ||
      snapshot.runtime.registry_busy ||
      !snapshot.runtime.selected_model
    ) {
      this.report(
        new DesktopError(
          "model_not_ready",
          "请先在模型页加载模型，等待运行服务就绪。",
        ),
      );
      return false;
    }
    let request: ChatRequest;
    try {
      request = checkSubmission(
        this.state.messages,
        text.trim(),
        snapshot.runtime.selected_model,
        snapshot.settings.max_output_tokens,
      );
    } catch (error) {
      this.report(error);
      return false;
    }
    const stream = {
      id: null as string | null,
      cancel: false,
      cancelSent: false,
      reading: false,
      capped: false,
      incompleteReason: null as string | null,
    };
    this.stream = stream;
    this.update({
      error: null,
      notice: null,
      chat_phase: "starting",
      messages: [
        ...this.state.messages,
        {
          id: ++this.nextMessage,
          role: "user",
          content: text.trim(),
          state: "complete",
        },
        {
          id: ++this.nextMessage,
          role: "assistant",
          content: "",
          state: "streaming",
        },
      ],
    });
    try {
      const { request_id } = await this.api.chatStart(request);
      if (!request_id)
        throw new DesktopError(
          "invalid_stream",
          "运行服务未返回有效请求标识。",
        );
      stream.id = request_id;
      this.update({ chat_phase: stream.cancel ? "stopping" : "streaming" });
      if (stream.cancel) void this.sendCancel();
      void this.consume();
    } catch (error) {
      this.markIncomplete("请求未能开始。");
      this.report(error);
      this.finish();
    }
    return true;
  };
  private markIncomplete(notice: string) {
    this.update({
      messages: this.state.messages.map((message, index, messages) =>
        index === messages.length - 1 && message.role === "assistant"
          ? { ...message, state: "incomplete", notice }
          : message,
      ),
    });
  }
  private finish() {
    const clear = this.state.clear_pending;
    this.stream = null;
    this.update({
      chat_phase: "idle",
      clear_pending: false,
      ...(clear ? { messages: [], notice: "会话已清空。" } : {}),
    });
    void this.refresh();
  }
  private async sendCancel() {
    const stream = this.stream;
    if (!stream?.id || stream.cancelSent) return;
    stream.cancelSent = true;
    try {
      await this.api.chatCancel(stream.id);
    } catch (error) {
      // Keep consuming: an acknowledgement failure is never a terminal event.
      if (this.stream === stream) {
        stream.cancelSent = false;
        this.report(error);
      }
    }
  }
  cancel = async () => {
    if (!this.stream) return;
    this.stream.cancel = true;
    this.update({
      chat_phase:
        this.state.chat_phase === "recovery" ? "recovery" : "stopping",
    });
    await this.sendCancel();
  };
  clear = async () => {
    if (this.stream) {
      this.update({ clear_pending: true });
      await this.cancel();
    } else this.update({ messages: [], error: null, notice: "会话已清空。" });
  };
  removeIncomplete = () => {
    if (this.stream || this.state.messages.at(-1)?.state !== "incomplete")
      return;
    this.update({
      messages: this.state.messages.slice(0, -2),
      error: null,
      notice: "未完成轮次已移除。",
    });
  };
  recover = async () => {
    if (!this.stream?.id || this.stream.reading) return;
    this.update({ error: null, chat_phase: "stopping" });
    this.stream.cancel = true;
    await this.sendCancel();
    void this.consume();
  };
  private applyBatch(batch: ChatBatch): boolean {
    const stream = this.stream;
    if (!stream || batch.request_id !== stream.id)
      throw new DesktopError(
        "invalid_stream",
        "收到不匹配的请求数据，已停止接收。",
      );
    const current = this.state.messages.at(-1);
    if (!current || current.role !== "assistant")
      throw new DesktopError("invalid_stream", "会话状态不一致。");
    let text = "";
    let terminal:
      | Exclude<
          ChatBatch["events"][number],
          { type: "delta" } | { type: "started" }
        >
      | undefined;
    for (const event of batch.events) {
      if (terminal)
        throw new DesktopError("invalid_stream", "终态之后出现额外事件。");
      if (event.type === "delta") text += event.text;
      else if (event.type !== "started") terminal = event;
    }
    if (batch.terminal !== Boolean(terminal))
      throw new DesktopError(
        "invalid_stream",
        "生成终态数据不完整，未按成功处理。",
      );
    const addition = byteLength(text);
    const total = this.state.messages.reduce(
      (sum, message) => sum + byteLength(message.content),
      0,
    );
    if (
      !stream.capped &&
      (addition > LIMITS.batch ||
        byteLength(current.content) + addition > LIMITS.reply ||
        total + addition > LIMITS.session)
    ) {
      stream.capped = true;
      stream.incompleteReason = "已达到安全大小上限";
      stream.cancel = true;
      this.report(
        new DesktopError(
          "history_limit",
          "回复或会话达到安全大小上限，已请求停止。已有文本保留为不完整回复，请清空或移除本轮。",
        ),
      );
      this.update({ chat_phase: "stopping" });
      void this.sendCancel();
    }
    let next: SessionMessage = {
      ...current,
      content: stream.capped ? current.content : current.content + text,
    };
    if (terminal) {
      if (terminal.type === "completed" && !stream.capped)
        next = {
          ...next,
          state: "complete",
          finish_reason: terminal.finish_reason,
          usage: terminal.usage,
          notice:
            terminal.finish_reason === "length" ? "已达到输出预算" : undefined,
        };
      else
        next = {
          ...next,
          state: "incomplete",
          notice: stream.capped
            ? (stream.incompleteReason ?? "回复不完整")
            : terminal.type === "cancelled"
              ? "已停止生成"
              : terminal.type === "failed"
                ? terminal.message
                : "回复不完整",
        };
      if (terminal.type === "failed") this.report(terminal);
    }
    this.update({ messages: [...this.state.messages.slice(0, -1), next] });
    return Boolean(terminal);
  }
  private async consume() {
    const stream = this.stream;
    if (!stream?.id || stream.reading) return;
    stream.reading = true;
    try {
      while (this.stream === stream) {
        const start = performance.now();
        const batch = await this.api.chatNext(stream.id);
        // Backpressure: no next pull until this batch is rendered; <= 30 batches/s.
        await wait(Math.max(0, 34 - (performance.now() - start)));
        if (this.stream !== stream) return;
        if (this.applyBatch(batch)) {
          this.finish();
          return;
        }
      }
    } catch (error) {
      if (this.stream === stream) {
        stream.capped = true;
        stream.incompleteReason = "连接中断，回复可能缺失，未按完整回复处理";
        this.markIncomplete("连接中断，终态尚未确认。");
        this.report(error);
        this.update({ chat_phase: "recovery" });
        stream.cancel = true;
        void this.sendCancel();
      }
    } finally {
      stream.reading = false;
    }
  }
}
