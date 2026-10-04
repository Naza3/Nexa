import { DesktopError, safeError } from "./adapter";
import { localValidationLabel, validLocalValidation } from "./localValidation";
import type {
  CatalogEntry,
  DownloadOperation,
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
  download_source: "modelscope",
  context_size: 2048,
  threads: 2,
  batch_size: 128,
  max_output_tokens: 512,
  idle_unload_seconds: 300,
  close_runtime_on_exit: false,
};
const encoder = new TextEncoder();
export const byteLength = (value: string) => encoder.encode(value).byteLength;
// Native responses are still validated before accepting a committed outcome.
// New fields are optional only for the old complete/failed desktop protocol.
function validLibraryOperation(value: LibraryOperation): boolean {
  const integer = (n: number, maximum: number) => Number.isSafeInteger(n) && n >= 0 && n <= maximum;
  const failures = value.file_errors ?? [];
  const published = value.status === "completed" || value.status === "partial";
  if (!["running", "completed", "partial", "cancelled", "failed"].includes(value.status) ||
    !["checking", "enumerating", "verifying", "committing", "finished"].includes(value.phase) ||
    !integer(value.examined_entries, 1025) || !integer(value.candidate_files, 64) ||
    !integer(value.verified_files, value.candidate_files) || value.candidate_files > value.examined_entries || !Array.isArray(failures) || failures.length > 64 ||
    failures.some((failure) => !failure || typeof failure.file_name !== "string" ||
      !failure.file_name || /[/\\\0]/.test(failure.file_name) || byteLength(failure.file_name) > 1024 ||
      !["invalid_manifest", "unsupported_model", "unsupported_chat_template"].includes(failure.code) ||
      typeof failure.message !== "string" || byteLength(failure.message) > 500) ||
    new Set(failures.map((failure) => failure.file_name)).size !== failures.length ||
    byteLength(JSON.stringify(failures)) > 512 * 1024 || byteLength(JSON.stringify(value)) > 1024 * 1024 ||
    value.verified_files + failures.length > value.candidate_files ||
    (value.status === "running" && (value.result !== null || value.error !== null)) ||
    (!published && value.result !== null) ||
    (value.status === "failed" && !value.error) ||
    (value.status === "cancelled" && value.error !== null && value.error?.code !== "model_scan_cancelled")) return false;
  if (published) {
    const result = value.result;
    const rejected = result?.rejected_files ?? 0;
    if (!result || value.error || !result.directory_id || !result.library_generation ||
      !integer(result.registered_files, 64) || !integer(result.available_files, result.registered_files) ||
      !integer(rejected, 64) || rejected !== failures.length || result.registered_files !== value.verified_files ||
      value.candidate_files !== result.registered_files + rejected || value.examined_entries > 1024 ||
      (value.status === "partial" && (!result.registered_files || !rejected)) ||
      (value.status === "completed" && rejected !== 0)) return false;
  }
  if (value.error?.code === "model_scan_no_usable_files" &&
    (value.status !== "failed" || value.verified_files !== 0 || failures.length === 0 || failures.length !== value.candidate_files)) return false;
  return true;
}
function validDownloadOperation(value: DownloadOperation): boolean {
  const text = (value: unknown, max: number, nonempty = true) => typeof value === "string" && (!nonempty || value.length > 0) && byteLength(value) <= max && !value.includes("\0");
  const bytes = (value: unknown) => typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value <= 16 * 1024 ** 3;
  if (!value || typeof value !== "object" || byteLength(JSON.stringify(value)) > 64 * 1024 ||
      !text(value.operation_id, 128) || !text(value.catalog_id, 256) || !text(value.directory_id, 128) ||
      !text(value.file_name, 1024) || /[/\\]/.test(value.file_name) || !text(value.target_display_path, 32768) ||
      (value.attempt !== undefined && (!Number.isSafeInteger(value.attempt) || value.attempt < 1 || value.attempt > 3)) ||
      !bytes(value.downloaded_bytes) || (value.total_bytes !== null && (!bytes(value.total_bytes) || value.total_bytes === 0)) ||
      typeof value.terminal !== "boolean" ||
      (value.error !== null && (!value.error || typeof value.error !== "object" ||
        !text(value.error.code, 80) || !/^[a-z0-9_]+$/.test(value.error.code) || !text(value.error.message, 500))) ||
      (value.result !== null && (!value.result || typeof value.result !== "object" ||
        value.result.saved !== true || typeof value.result.registered !== "boolean" || !text(value.result.file_name, 1024) ||
        (value.result.registration_error != null && (!text(value.result.registration_error.code, 80) || !/^[a-z0-9_]+$/.test(value.result.registration_error.code) || !text(value.result.registration_error.message, 500))) ||
        (value.result.registered && value.result.registration_error != null) ||
        (value.result.local_validation != null && (!value.result.registered || !validLocalValidation(value.result.local_validation))) ||
        (value.result.cleanup_warning !== null && !text(value.result.cleanup_warning, 500, false))))) return false;
  return true;
}
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
  catalog: CatalogEntry[];
  catalog_loading: boolean;
  catalog_loaded: boolean;
  download: DownloadOperation | null;
  download_phase: "idle" | "starting" | "running" | "stopping" | "recovery";
  download_auto_test: boolean;
  discovery: "unchecked" | "checking" | "none" | "configured" | "failed";
  reconcile_status: "idle" | "checking" | "observing" | "pending" | "failed";
  testing_model: string | null;
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
  if (!["modelscope", "huggingface"].includes(settings.download_source)) return "请选择有效的默认下载源。";
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
    catalog: [], catalog_loading: false, catalog_loaded: false,
    download: null, download_phase: "idle", download_auto_test: false, discovery: "unchecked",
    reconcile_status: "idle", testing_model: null,
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
  private reconcileTimer: ReturnType<typeof setTimeout> | undefined;
  private initialReconcile = false;
  private lastReconcile = -Infinity;
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
  private downloadTask: { id: string | null; catalog_id: string; source: Settings["download_source"]; directory_id: string; cancel: boolean; cancelSent: boolean; reading: boolean; lastPull: number } | null = null;
  private modelsEpoch = 0;
  private nextMessage = 0;
  private modelsLoaded = false;
  private closing = false;
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
      if (this.mounted && epoch === this.pollEpoch && this.state.discovery === "unchecked" && this.state.snapshot) {
        if (this.state.snapshot.model_directory.configured) this.update({ discovery: "configured" });
        else if (this.state.snapshot.connection === "stopped") await this.discoverDirectory();
      }
      if (this.mounted && epoch === this.pollEpoch && !this.initialReconcile && this.state.snapshot?.model_directory.configured) {
        this.initialReconcile = true;
        void this.reconcileModels();
      }
      if (this.mounted && epoch === this.pollEpoch)
        this.poll = setTimeout(tick, 1000);
    };
    void tick();
    return () => {
      if (epoch !== this.pollEpoch) return;
      this.mounted = false;
      clearTimeout(this.poll);
      clearTimeout(this.reconcileTimer);
      void this.cancel();
      void this.cancelLibrary();
      void this.cancelDownload();
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
          ["connected", "stopped"].includes(snapshot.connection) &&
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
      this.closing || this.state.operation ||
      (!allowChat && this.stream) ||
      (!allowLibrary && (this.libraryTask || this.downloadTask))
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
      !this.state.snapshot || !["connected", "stopped"].includes(this.state.snapshot.connection) ||
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
            if (!Array.isArray(models.data) || models.data.length > 64 || !models.generation ||
                (models.source !== undefined && !["local", "runtime"].includes(models.source)) ||
                models.data.some((model) => model.local_validation != null && !validLocalValidation(model.local_validation)))
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
  discoverDirectory = async () => {
    if (this.state.snapshot?.model_directory.configured) {
      this.update({ discovery: "configured" });
      return;
    }
    if (this.state.snapshot?.connection !== "stopped" || this.libraryTask || this.downloadTask || this.state.operation || this.stream) return;
    this.update({ discovery: "checking" });
    await this.beginLibrary("discover");
  };
  applyDirectory = () => this.beginLibrary("apply");
  scanModels = () => this.beginLibrary("scan");
  reconcileModels = async (observeAgain = true) => {
    if (!this.state.snapshot?.model_directory.configured || this.libraryTask || this.downloadTask || this.state.operation || this.stream) return;
    if (performance.now() - this.lastReconcile < 1000) return;
    this.lastReconcile = performance.now();
    clearTimeout(this.reconcileTimer);
    await this.beginLibrary("reconcile");
    // One bounded second observation, never a background full-hash polling loop.
    if (observeAgain && this.state.reconcile_status === "observing" && (this.mounted || this.pollEpoch === 0)) {
      // Native stability window is two seconds; allow scheduling margin.
      this.reconcileTimer = setTimeout(() => { void this.reconcileModels(false); }, 2200);
    }
  };
  refreshModels = async () => {
    await this.reconcileModels();
    if (!this.libraryTask) await this.loadPage(null);
  };
  private async beginLibrary(kind: "apply" | "scan" | "discover" | "reconcile") {
    if (this.libraryTask || this.downloadTask || this.state.operation || this.stream) return;
    if (kind !== "reconcile" && this.state.snapshot?.connection !== "stopped") {
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
    if (kind === "scan" && !this.state.snapshot?.model_directory.configured) {
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
      ...(kind === "reconcile" ? { reconcile_status: "checking" as const } : {}),
    });
    try {
      let handle;
      if (kind === "reconcile") {
        const result = await this.api.reconcileModels();
        if (!["unchanged", "observing", "pending", "started"].includes(result.status) ||
            (result.status === "started" ? !result.operation_id : result.operation_id !== null))
          throw new DesktopError("invalid_reconcile_result", "目录检查返回了无效状态，未自动重试。");
        if (result.status !== "started") {
          this.libraryTask = null;
          ++this.snapshotEpoch;
          this.update({ library_phase: "idle", reconcile_status: result.status === "unchanged" ? "idle" : result.status });
          // Initial model-page effects can overlap the first snapshot read.
          // A no-op inspection must not leave offline inventory blank until a poll.
          if (!this.modelsLoaded) {
            if (this.modelsPromise) await this.modelsPromise;
            await this.loadPage(null);
          }
          return;
        }
        handle = result;
      } else handle = kind === "discover"
          ? await this.api.discoverDirectory()
          : kind === "apply"
          ? await this.api.applyDirectory(selection!.selection_id)
          : await this.api.scanModels();
      if (kind === "discover" && !handle) {
        this.libraryTask = null;
        ++this.snapshotEpoch;
        this.update({ library_phase: "idle", discovery: "none" });
        return;
      }
      if (!handle?.operation_id)
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
      this.update({ library_phase: "idle", ...(kind === "discover" ? { discovery: "failed" as const } : {}), ...(kind === "reconcile" ? { reconcile_status: "failed" as const } : {}) });
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
          !validLibraryOperation(progress) || progress.operation_id !== task.id ||
          progress.terminal !== terminal ||
          (terminal && progress.phase !== "finished") ||
          ((progress.status === "completed" || progress.status === "partial") &&
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
        if (this.state.reconcile_status === "checking") this.update({ reconcile_status: progress.status === "failed" ? "failed" : "idle" });
        if (this.state.discovery === "checking") this.update({ discovery: progress.status === "completed" || progress.status === "partial" ? "configured" : "failed" });
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
        if (progress.status === "completed" || progress.status === "partial") {
          // Scan diagnostics now supersede the historical saved-but-unregistered download notice.
          // A partial scan does not prove this particular file was accepted.
          if (this.state.download?.status === "completed" && progress.result?.directory_id === this.state.download.directory_id)
            this.update({ download: null });
          ++this.modelsEpoch;
          this.modelsLoaded = false;
          this.update({
            models: { data: [], next_after: null, generation: null },
            page_after: null,
            notice: progress.status === "partial" ? null : progress.result!.registered_files === 0
              ? "模型目录已保存，未发现直接子级 GGUF 文件；当前外部索引为空。"
              : `模型目录已保存，登记 ${progress.result!.registered_files} 个文件，其中 ${progress.result!.available_files} 个可尝试加载（不代表已实测）。本机测试结果以模型列表记录为准。`,
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
  loadCatalog = async () => {
    if (this.state.catalog_loading) return;
    this.update({ catalog_loading: true });
    try {
      const result = await this.api.catalog();
      const bounded = (value: unknown) => typeof value === "string" && byteLength(value) <= 8192;
      if (!Array.isArray(result.entries) || result.entries.length > 128 || byteLength(JSON.stringify(result)) > 1024 * 1024 ||
          result.entries.some((entry) => !entry || !entry.catalog_id || !entry.file_name ||
            ![entry.catalog_id, entry.file_name, entry.display_name, entry.architecture, entry.quantization, entry.sha256, entry.license, entry.recommendation].every(bounded) ||
            !Number.isSafeInteger(entry.context_hint) || entry.context_hint < 0 ||
            !Number.isSafeInteger(entry.size_bytes) || entry.size_bytes < 0 ||
            !Array.isArray(entry.sources) || entry.sources.some((source) =>
              !source || !["modelscope", "huggingface"].includes(source.source) ||
              ![source.repository, source.revision, source.url].every(bounded))) ||
          new Set(result.entries.map((entry) => entry.catalog_id)).size !== result.entries.length)
        throw new DesktopError("invalid_model_catalog", "下载目录数据无效，请重试读取。");
      this.update({ catalog: result.entries, catalog_loaded: true });
    } catch (error) { this.report(error); }
    finally { this.update({ catalog_loading: false }); }
  };
  startDownload = async (catalog_id: string, auto_test?: boolean) => {
    if (this.downloadTask || this.libraryTask || this.stream || this.state.operation) return;
    const snapshot = this.state.snapshot;
    const entry = this.state.catalog.find((entry) => entry.catalog_id === catalog_id);
    if (!snapshot || snapshot.connection !== "stopped" || !snapshot.model_directory.configured) {
      this.report(new DesktopError("model_directory_required", "请先停止服务并选择、应用模型目录，再下载。"));
      return;
    }
    if (!entry?.sources.some((source) => source.source === snapshot.settings.download_source)) {
      this.report(new DesktopError("download_source_unavailable", "此模型在已保存的下载源不可用，请在设置中选择其他来源并保存。"));
      return;
    }
    const task = { id: null as string | null, catalog_id, source: snapshot.settings.download_source, directory_id: snapshot.model_directory.configured.directory_id, cancel: false, cancelSent: false, reading: false, lastPull: -Infinity };
    this.downloadTask = task;
    this.update({ download: null, download_phase: "starting", download_auto_test: auto_test === true, error: null, notice: null });
    try {
      const handle = auto_test === undefined ? await this.api.downloadStart(catalog_id) : await this.api.downloadStart(catalog_id, auto_test);
      if (!handle.operation_id) throw new DesktopError("invalid_download_operation", "下载未返回有效标识，未自动重试。");
      task.id = handle.operation_id;
      this.update({ download_phase: task.cancel ? "stopping" : "running" });
      if (task.cancel) void this.sendDownloadCancel();
      void this.consumeDownload();
    } catch (error) {
      this.downloadTask = null;
      this.update({ download_phase: "idle" });
      this.report(error);
    }
  };
  cancelDownload = async () => {
    if (!this.downloadTask) return;
    this.downloadTask.cancel = true;
    if (this.state.download_phase !== "recovery") this.update({ download_phase: "stopping" });
    await this.sendDownloadCancel();
  };
  private async sendDownloadCancel() {
    const task = this.downloadTask;
    if (!task?.id || task.cancelSent) return;
    task.cancelSent = true;
    try { await this.api.downloadCancel(task.id); }
    catch (error) {
      if (this.downloadTask === task) { task.cancelSent = false; this.report(error); }
    }
  }
  recoverDownload = async () => {
    if (!this.downloadTask?.id || this.downloadTask.reading) return;
    this.update({ download_phase: "stopping", error: null });
    this.downloadTask.cancel = true;
    await this.sendDownloadCancel();
    void this.consumeDownload();
  };
  private async consumeDownload() {
    const task = this.downloadTask;
    if (!task?.id || task.reading) return;
    task.reading = true;
    try {
      while (this.downloadTask === task) {
        await wait(Math.max(0, Math.ceil(1000 - (performance.now() - task.lastPull))));
        if (this.downloadTask !== task) return;
        if (performance.now() - task.lastPull < 1000) continue;
        task.lastPull = performance.now();
        const value = await this.api.downloadNext(task.id);
        if (this.downloadTask !== task) return;
        if (!validDownloadOperation(value))
          throw new DesktopError("invalid_download_operation", "下载状态字段无效，尚未确认操作结束。");
        const terminal = value.status !== "running";
        if (value.operation_id !== task.id || value.catalog_id !== task.catalog_id || value.source !== task.source || value.directory_id !== task.directory_id ||
            !["running", "completed", "cancelled", "failed"].includes(value.status) ||
            !["connecting", "downloading", "verifying", "committing", "registering", "testing", "finished"].includes(value.phase) ||
            !["modelscope", "huggingface"].includes(value.source) ||
            !Number.isSafeInteger(value.downloaded_bytes) || value.downloaded_bytes < 0 ||
            (value.total_bytes !== null && (!Number.isSafeInteger(value.total_bytes) || value.total_bytes < value.downloaded_bytes)) ||
            value.terminal !== terminal || (terminal && value.phase !== "finished") ||
            (!terminal && (value.result !== null || value.error !== null)) ||
            (value.status === "completed" && (!value.result?.saved || value.result.file_name !== value.file_name || value.error !== null ||
              (value.total_bytes !== null && value.downloaded_bytes !== value.total_bytes))) ||
            (value.status !== "completed" && value.result !== null) ||
            (value.status === "failed" && !value.error))
          throw new DesktopError("invalid_download_operation", "下载状态数据无效，尚未确认下载结束。请重新确认，勿重复下载。");
        const attempt = value.attempt ?? 1;
        const previous = this.state.download;
        if (previous && (previous.source !== value.source || previous.directory_id !== value.directory_id ||
            previous.file_name !== value.file_name || previous.target_display_path !== value.target_display_path ||
            (previous.total_bytes !== null && previous.total_bytes !== value.total_bytes) ||
            attempt < (previous.attempt ?? 1) ||
            (attempt === (previous.attempt ?? 1) && value.downloaded_bytes < previous.downloaded_bytes)))
          throw new DesktopError("invalid_download_operation", "下载身份或进度发生异常，尚未确认下载结束。");
        this.update({ download: { ...value, attempt } });
        if (!terminal) continue;
        this.downloadTask = null;
        this.update({ download_phase: "idle" });
        if (value.status === "completed") {
          this.update({ notice: `${value.result?.registered
            ? "模型文件已保存并自动登记。本机测试结果以模型列表记录为准。"
            : value.result?.registration_error
              ? "模型文件已保存，但自动登记未完成；请勿重复下载，可检查登记诊断后重试扫描。"
              : "模型文件已保存，尚未登记。请重新扫描目录。"}${value.result?.cleanup_warning ? "部分下载文件清理未确认，请勿重复下载。" : ""}` });
          ++this.modelsEpoch;
          this.modelsLoaded = false;
          if (this.modelsPromise) await this.modelsPromise;
          if (this.snapshotPromise) await this.snapshotPromise;
          await this.refresh();
        }
        else if (value.status === "cancelled") this.update({ notice: "下载已取消。" });
        else this.report(value.error);
        return;
      }
    } catch (error) {
      if (this.downloadTask === task) {
        this.update({ download_phase: "recovery" });
        this.report(error);
        task.cancel = true;
        void this.sendDownloadCancel();
      }
    } finally { task.reading = false; }
  }
  private setRuntime(runtime: RuntimeStatus) {
    if (this.state.snapshot)
      this.update({ snapshot: { ...this.state.snapshot, runtime } });
  }
  private async refreshModelEvidence() {
    ++this.modelsEpoch;
    this.modelsLoaded = false;
    if (this.modelsPromise) await this.modelsPromise;
    if (this.snapshotPromise) await this.snapshotPromise;
    await this.refresh();
    if (!this.modelsLoaded) await this.loadPage(null);
  }
  loadModel = (modelId: string) =>
    this.action("正在加载并进行本机基础测试", async () => {
      const model = this.state.models.data.find((entry) => entry.id === modelId);
      if (!model?.available || model.loadable !== true)
        throw new Error("当前模型不可尝试加载，请刷新匹配版本的模型列表");
      const snapshot = this.state.snapshot;
      if (snapshot?.connection === "stopped") {
        // Only this explicit load action may start the service; listing never does.
        this.update({ snapshot: await this.api.start(!snapshot.initialized) });
        ++this.modelsEpoch;
        this.modelsLoaded = false;
      }
      const settings = this.state.snapshot?.settings ?? DEFAULT_SETTINGS;
      this.update({ testing_model: modelId });
      try {
        this.setRuntime(await this.api.loadModel(modelId, {
          context_size: settings.context_size,
          threads: settings.threads,
          batch_size: settings.batch_size,
        }));
        await this.refreshModelEvidence();
        const evidence = this.state.models.data.find((entry) => entry.id === modelId)?.local_validation;
        this.update({ notice: evidence ? localValidationLabel(evidence) : "模型加载操作已结束，尚未取得本机基础测试记录，请刷新查看。" });
      } catch (error) {
        await this.refreshModelEvidence();
        throw error;
      } finally { this.update({ testing_model: null }); }
    });
  testModel = (modelId: string) =>
    this.action("正在进行本机短文本测试", async () => {
      const options = this.state.snapshot?.runtime?.load_options;
      if (!options || this.state.snapshot?.runtime?.selected_model !== modelId)
        throw new DesktopError("model_not_ready", "请先显式加载此模型再测试。");
      this.update({ testing_model: modelId });
      try {
        const result = await this.api.testModel(modelId, options);
        if (!validLocalValidation(result)) throw new DesktopError("invalid_local_validation", "测试返回的记录无效，未按通过处理。");
        // Never attach a late response to a changed file, page, engine or options.
        await this.refreshModelEvidence();
        const evidence = this.state.models.data.find((entry) => entry.id === modelId)?.local_validation;
        this.update({ notice: result.state !== "passed"
          ? `${localValidationLabel(result)}${evidence?.state === "passed" ? "；列表中的通过标签来自此前记录。" : ""}`
          : evidence ? localValidationLabel(evidence) : "本机测试已结束，请查看对应模型的最新本机记录。" });
      } finally { this.update({ testing_model: null }); }
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
        const previous = this.state.snapshot?.settings;
        const optionsChanged = !previous || (["context_size", "threads", "batch_size"] as const).some((key) => previous[key] !== settings[key]);
        this.update({
          snapshot: await this.api.saveSettings(settings),
          notice:
            "偏好已保存。加载参数在下次加载时生效，输出预算用于下次发送。",
        });
        if (optionsChanged) {
          // Old proof stays historical even if the authoritative reread fails.
          this.update({ models: { ...this.state.models, data: this.state.models.data.map((model) => ({
            ...model, ...(model.local_validation ? { local_validation: { ...model.local_validation, state: "stale" as const } } : {}),
          })) } });
          ++this.modelsEpoch;
          this.modelsLoaded = false;
          if (this.modelsPromise) await this.modelsPromise;
          await this.loadPage(null);
        }
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
  close = async () => {
    if (this.closing || (this.state.operation && !this.state.testing_model)) return;
    this.closing = true;
    try { await this.api.close(); }
    catch (error) { this.report(error); }
    finally { this.closing = false; }
  };
  send = async (text: string): Promise<boolean> => {
    if (this.stream || this.libraryTask || this.downloadTask || this.state.operation || !text.trim())
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
