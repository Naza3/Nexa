import { projectActivities, recordActivity, readActivityHistory, createActivitySessionId, persistActivitySummaries } from "./activity";
import { validModelLoadOperation, validOptionalLoadPhase } from "./modelLoad";
import { modelRemovalBlocker } from "./modelRemoval";
import { ownRecord } from "./records";
import type { Activity } from "./activity";
import { localBaseUrl } from "./runtimeView";
import { DesktopError, safeError } from "./adapter";
import { localValidationLabel, unavailableValidation, validLocalValidation, validationErrorReason } from "./localValidation";
import { validAddOperation, validModelSelection } from "./modelSelection";
import { lanBaseUrl, validateLanSettings, validLanAddresses } from "./lanApi";
import { preferencesOnly, validateIdleSeconds, validateVerificationSeconds } from "./runtimeSettingsValues";
import type {
  ConfigurationSnapshot, ConfigurationSaveRequest, ConfigurationMigrateRequest, ModelConfiguration, UiPreferences,
  CatalogEntry,
  DownloadOperation,
  ChatBatch,
  ChatRequest,
  DesktopApi,
  ModelPage,
  ModelSummary,
  ModelLoadOperation,
  LocalValidation,
  LoadOptions,
  DirectorySelection,
  ModelFileSelection,
  LibraryOperation,
  LanApiSettings,
  LanAddressDiscovery,
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
  idle_unload_enabled: true,
  model_verification_timeout_seconds: 300,
  close_runtime_on_exit: false,
};
const encoder = new TextEncoder();
export const byteLength = (value: string) => encoder.encode(value).byteLength;
// Native responses are still validated before accepting a committed outcome.
// New fields are optional only for the old complete/failed desktop protocol.
function validLibraryOperation(value: LibraryOperation, configuring = false): boolean {
  const integer = (n: number, maximum: number) => Number.isSafeInteger(n) && n >= 0 && n <= maximum;
  const failures = value.file_errors ?? [];
  if (configuring && (value.examined_entries !== 0 || value.candidate_files !== 0 || value.verified_files !== 0 || failures.length || value.status === "partial" || !["checking", "committing", "finished"].includes(value.phase))) return false;
  const published = value.status === "completed" || value.status === "partial";
  if (!validOptionalLoadPhase(value.load_phase) || !["running", "completed", "partial", "cancelled", "failed"].includes(value.status) ||
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
      !integer(rejected, 64) || rejected !== failures.length || (!configuring && result.registered_files !== value.verified_files) ||
      (!configuring && value.candidate_files !== result.registered_files + rejected) || value.examined_entries > 1024 ||
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
  if (!value || typeof value !== "object" || !validOptionalLoadPhase(value.load_phase) || byteLength(JSON.stringify(value)) > 64 * 1024 ||
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
export interface ModelTestAttempt {
  id: number;
  model_id: string;
  model_signature: string;
  mode: "load" | "test";
  phase: "running" | "finished";
  outcome?: "cancelled";
  started_at: number;
  finished_at: number | null;
  result: LocalValidation | null;
  error: SafeError | null;
}
interface ModelTestScope {
  attempt: ModelTestAttempt;
  epoch: number;
  page_after: string | null;
  options: LoadOptions;
  settings: string;
  backend: string | null;
  connection: Snapshot["connection"] | null;
  selected_model: string | null;
  runtime_options: string | null;
}
// Local evidence may change on reread without changing the model's identity.
function modelSignature(model: ModelSummary): string {
  return JSON.stringify([model.id, model.sha256, model.size_bytes, model.storage, model.architecture, model.quantization,
    model.available, model.availability_error, model.loadable, model.context_limit, model.context_size]);
}
function loadOptions(value: LoadOptions): string {
  return JSON.stringify([value.context_size, value.threads, value.batch_size]);
}
export interface CommandOperation { id: number; kind: "start" | "stop" | "check" | "pick_models" | "read_models" | "remove_model" | "configuration" | "initialize" | "management"; label: string }
export interface ModelLoadTaskView {
  attempt_id: number;
  model_id: string;
  operation_id: string | null;
  phase: "starting" | "running" | "stopping" | "recovery";
  progress: ModelLoadOperation | null;
  cancel_error: SafeError | null;
  stop_requested: boolean;
}
interface ModelLoadTask {
  view: ModelLoadTaskView;
  cancel: boolean;
  cancelSent: boolean;
  cancelPending: boolean;
  accepted: boolean;
  startSettled: boolean;
  resume: (() => void) | null;
}
export interface ViewState {
  model_removal: { model_id: string; id: number } | null;
  model_load: ModelLoadTaskView | null;
  activities: Activity[];
  activity_session_id: string;
  activity_storage_warning: string | null;
  configuration_recovery_error: SafeError | null;
  model_configurations: Record<string, ModelConfiguration>;
  model_configuration_errors: Record<string, SafeError>;
  lan_addresses: LanAddressDiscovery | null;
  lan_addresses_loading: boolean;
  lan_addresses_error: SafeError | null;
  catalog: CatalogEntry[];
  catalog_loading: boolean;
  catalog_loaded: boolean;
  download: DownloadOperation | null;
  download_task_id: string | null;
  library_task_id: string | null;
  download_phase: "idle" | "starting" | "running" | "stopping" | "recovery";
  download_auto_test: boolean;
  discovery: "unchecked" | "checking" | "none" | "configured" | "failed";
  reconcile_status: "idle" | "checking" | "observing" | "pending" | "failed";
  testing_model: string | null;
  model_tests: Record<string, ModelTestAttempt>;
  snapshot: Snapshot | null;
  booting: boolean;
  error: SafeError | null;
  notice: string | null;
  operation: CommandOperation | null;
  models: Omit<ModelPage, "generation"> & { generation: string | null };
  page_after: string | null;
  models_loading: boolean;
  directory_selection: DirectorySelection | null;
  model_selection: ModelFileSelection | null;
  add_auto_test: boolean;
  library_kind: "maintenance" | "add" | "configure";
  library_selection: ModelFileSelection | null;
  library: LibraryOperation | null;
  library_phase: "idle" | "starting" | "running" | "stopping" | "recovery";
  messages: SessionMessage[];
  chat_phase: ChatPhase;
  clear_pending: boolean;
}
export function canDismissAddResult(state: ViewState): boolean {
  return state.library_kind === "add" && state.library_phase === "idle" &&
    state.library?.terminal === true && state.library.phase === "finished" &&
    ["completed", "partial", "cancelled", "failed"].includes(state.library.status);
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
  private activityHistory = readActivityHistory();
  private state: ViewState = {
    model_removal: null,
    model_load: null,
    activities: this.activityHistory.records,
    activity_session_id: createActivitySessionId(),
    activity_storage_warning: this.activityHistory.warning,
    configuration_recovery_error: null,
    model_configurations: {}, model_configuration_errors: {},
    lan_addresses: null, lan_addresses_loading: false, lan_addresses_error: null,
    catalog: [], catalog_loading: false, catalog_loaded: false,
    download: null, download_task_id: null, library_task_id: null, download_phase: "idle", download_auto_test: false, discovery: "unchecked",
    reconcile_status: "idle", testing_model: null, model_tests: {},
    snapshot: null,
    booting: true,
    error: null,
    notice: null,
    operation: null,
    models: { data: [], next_after: null, generation: null },
    page_after: null,
    models_loading: false,
    directory_selection: null,
    model_selection: null, add_auto_test: false, library_kind: "maintenance", library_selection: null,
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
  private lastReconcile = -Infinity;
  private pollEpoch = 0;
  private modelsPromise: Promise<void> | null = null;
  private lanAddressesPromise: Promise<void> | null = null;
  private snapshotPromise: Promise<void> | null = null;
  private snapshotEpoch = 0;
  private configurationReadEpoch = 0;
  private stream: {
    id: string | null;
    cancel: boolean;
    cancelSent: boolean;
    reading: boolean;
    capped: boolean;
    incompleteReason: string | null;
    terminal: "completed" | "cancelled" | "failed" | null;
    error: SafeError | null;
  } | null = null;
  private libraryTask: {
    id: string | null;
    cancel: boolean;
    cancelSent: boolean;
    reading: boolean;
    lastPull: number;
  } | null = null;
  private downloadTask: { id: string | null; catalog_id: string; source: Settings["download_source"]; directory_id: string; cancel: boolean; cancelSent: boolean; reading: boolean; lastPull: number } | null = null;
  private selectionEpoch = 0;
  private modelsEpoch = 0;
  private modelRemovalPending = false;
  private nextMessage = 0;
  private nextOperation = 0;
  private nextTask = 0;
  private modelConfigurationReads = new Map<string, number>();
  private modelsLoaded = false;
  private closing = false;
  private closeDecision: Promise<void> | null = null;
  private modelFeedbackEpoch = 0;
  private settingsEpoch = 0;
  private nextTest = 0;
  private modelLoadTask: ModelLoadTask | null = null;
  private snapshotReadError: SafeError | null = null;
  private modelsReadError: SafeError | null = null;
  constructor(readonly api: DesktopApi) {}
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private update(patch: Partial<ViewState>, persist = true) {
    const next = { ...this.state, ...patch };
    const activities = projectActivities(next.activities, next);
    const warning = persist && activities !== this.state.activities ? persistActivitySummaries(activities) : next.activity_storage_warning;
    this.state = { ...next, activities, activity_storage_warning: warning };
    this.listeners.forEach((listener) => listener());
  }
  dismissError = () => this.update({ error: null });
  dismissNotice = (notice: string) => {
    if (this.state.notice === notice) this.update({ notice: null });
  };
  private report(error: unknown) {
    const safe = safeError(error);
    const messages: Record<string, string> = {
      configuration_conflict: "配置已被其他窗口修改。草稿保留，请重新读取并核对后再保存；未自动重试。",
      configuration_durability_unconfirmed: "配置可能已发布，但持久化尚未确认。请重新读取核对，不要直接重复保存。",
      configuration_restart_required: "已保存配置与当前运行实例不一致，请先显式停止并重新启动服务。",
      configuration_migration_required: "请先在设置中确认旧配置迁移来源。",
      configuration_unavailable: "当前后台无法提供统一配置。请检查版本；必要时显式停止并重新启动匹配版本，不会自动替换实例。",
    };
    this.update({ error: { ...safe, message: messages[safe.code] ?? safe.message } });
  }
  mount = () => {
    ++this.settingsEpoch;
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
      ++this.settingsEpoch;
      ++this.modelFeedbackEpoch;
      this.update({ model_tests: {} });
      clearTimeout(this.poll);
      clearTimeout(this.reconcileTimer);
      void this.discardModelSelection();
      void this.cancel();
      void this.cancelModelLoad();
      void this.cancelLibrary();
      void this.cancelDownload();
    };
  };
  refresh = (modelAfter: string | null = null): Promise<void> => {
    if (this.snapshotPromise) return this.snapshotPromise;
    const epoch = this.snapshotEpoch;
    this.snapshotReadError = null;
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
        ++this.configurationReadEpoch;
        this.update({ snapshot, booting: false, configuration_recovery_error: null });
        if (
          ["connected", "stopped"].includes(snapshot.connection) &&
          !["stale", "unsupported"].includes(snapshot.model_directory.state) &&
          !this.libraryTask &&
          !this.modelRemovalPending &&
          !this.modelsLoaded &&
          !this.state.models_loading
        )
          await this.loadPage(modelAfter);
      } catch (error) {
        if (epoch !== this.snapshotEpoch) return;
        this.snapshotReadError = safeError(error);
        this.update({
          booting: false,
          configuration_recovery_error: ["configuration_invalid", "configuration_unavailable", "settings_invalid"].includes(this.snapshotReadError.code) ? this.snapshotReadError : this.state.configuration_recovery_error,
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
  /** Reads already in flight before an acknowledgement cannot become post-write truth. */
  private async refreshAfterMutation() {
    ++this.snapshotEpoch;
    const pending = this.snapshotPromise;
    if (pending) await pending;
    ++this.snapshotEpoch;
    await this.refresh();
  }
  private async action(
    label: string,
    task: () => Promise<void | "recovery">,
    allowChat = false,
    allowLibrary = false,
    kind: CommandOperation["kind"] = "management",
  ) {
    if (
      this.closing || this.state.operation ||
      (!allowChat && this.stream) ||
      (!allowLibrary && (this.libraryTask || this.downloadTask))
    )
      return;
    ++this.snapshotEpoch;
    const operation = { id: ++this.nextOperation, kind, label };
    const activityKind: Activity["kind"] | null = ["start", "stop", "initialize"].includes(kind) ? "service" : kind === "configuration" ? "configuration" : null;
    const id = `${this.state.activity_session_id}:action:${operation.id}`;
    this.update({ operation, error: null, notice: null, ...(activityKind ? { activities: recordActivity(this.state.activities, { id, kind: activityKind, label: label.replace(/^正在/, ""), status: "running", updated_at: Date.now(), detail: "正在执行已确认操作；没有单独取消能力的阶段需等待原生结果。", error: null }) } : {}) });
    let failure: SafeError | null = null;
    let outcome: void | "recovery" = undefined;
    try {
      outcome = await task();
      // A rejected concurrent settings attempt is no longer busy after this action succeeds.
      if (this.state.error?.code === "operation_in_progress") this.update({ error: null });
    } catch (error) {
      failure = safeError(error);
      this.report(error);
    } finally {
      ++this.snapshotEpoch;
      this.update({ operation: null, ...(activityKind ? { activities: recordActivity(this.state.activities, { id, kind: activityKind, label: label.replace(/^正在/, ""), status: failure ? "failed" : outcome === "recovery" ? "recovery" : "completed", updated_at: Date.now(), detail: failure ? "操作未完成，请核对诊断；未自动重放。" : outcome === "recovery" ? "原生指令已返回，但最新状态未核实。请重新检查服务与配置，不要直接重放。" : "操作已结束，当前监听、驻留与配置是否生效以最新快照为准。", error: failure }) } : {}) });
    }
  }
  start = (initialize: boolean) =>
    this.action(initialize ? "正在初始化并启动" : "正在启动服务", async () => {
      if (this.state.booting || this.state.snapshot?.connection !== "stopped") return;
      let started: Snapshot;
      try { started = await this.api.start(initialize); }
      catch (error) { this.markServiceUnknown(); throw error; }
      ++this.snapshotEpoch;
      this.update({ snapshot: started });
      if (this.snapshotPromise) await this.snapshotPromise;
      ++this.modelsEpoch;
      this.modelsLoaded = false;
      if (this.modelsPromise) await this.modelsPromise;
      await this.loadPage(null);
      await this.refresh();
    }, false, false, "start");
  loadPage = (after: string | null): Promise<void> => {
    if (after !== this.state.page_after) this.leaveModelPage();
    if (this.modelsPromise) return this.modelsPromise;
    if (
      !this.state.snapshot || !["connected", "stopped"].includes(this.state.snapshot.connection) ||
      this.libraryTask ||
      this.modelRemovalPending ||
      ["stale", "unsupported"].includes(
        this.state.snapshot.model_directory.state,
      )
    )
      return Promise.resolve();
    const epoch = this.modelsEpoch;
    this.modelsReadError = null;
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
                (models.source !== undefined && !["local", "runtime"].includes(models.source)))
              throw new DesktopError(
                "invalid_model_page",
                "模型列表缺少有效版本或超过单页上限。",
              );
            // A damaged per-model receipt must not hide an otherwise usable inventory.
            const data = models.data.map((model) => model.local_validation != null && !validLocalValidation(model.local_validation)
              ? { ...model, local_validation: unavailableValidation() } : model);
            const model_tests = Object.fromEntries(Object.entries(this.state.model_tests).filter(([id, attempt]) =>
              data.some((model) => model.id === id && modelSignature(model) === attempt.model_signature)));
            this.modelsLoaded = true;
            this.update({ models: { ...models, data }, page_after: cursor, model_tests });
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
        if (epoch === this.modelsEpoch) {
          this.modelsReadError = safeError(error);
          this.report(error);
        }
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
  pickModels = async () => {
    if (this.closing || this.state.operation || this.libraryTask || this.downloadTask || this.stream) return;
    const epoch = ++this.selectionEpoch;
    this.update({ operation: { id: ++this.nextOperation, kind: "pick_models", label: "正在选择 GGUF 文件" }, model_selection: null, add_auto_test: false, error: null, notice: null });
    try {
      const selection = await this.api.pickModels();
      if (epoch !== this.selectionEpoch || this.closing) {
        if (selection?.selection_id) await this.api.discardModelSelection(selection.selection_id);
        return;
      }
      if (selection && !validModelSelection(selection)) {
        if (typeof selection.selection_id === "string" && selection.selection_id.length <= 128)
          await this.api.discardModelSelection(selection.selection_id);
        throw new DesktopError("invalid_model_selection", "文件选择结果无效，请重新选择 GGUF 文件。");
      }
      this.update({ model_selection: selection, ...(selection ? {} : { notice: "已取消文件选择，现有模型索引未改变。" }) });
    } catch (error) {
      if (epoch === this.selectionEpoch) this.report(error);
    } finally {
      if (this.getSnapshot().operation?.kind === "pick_models") this.update({ operation: null });
    }
  };
  discardModelSelection = async () => {
    const selection = this.state.model_selection;
    ++this.selectionEpoch;
    this.update({ model_selection: null, add_auto_test: false });
    if (!selection) return;
    try { await this.api.discardModelSelection(selection.selection_id); }
    catch (error) { this.report(error); }
  };
  setAddAutoTest = (enabled: boolean) => {
    if (this.state.operation || this.libraryTask) return;
    this.update({ add_auto_test: enabled && this.state.model_selection?.files.length === 1 });
  };
  addModels = () => this.beginLibrary("add");
  dismissAddResult = (result: LibraryOperation) => {
    // Bind dismissal to the rendered result, so a stale click cannot clear a newer add.
    // Keep native ownership, inventory, new selections and unrelated feedback untouched.
    if (this.libraryTask || this.state.library !== result || !canDismissAddResult(this.state)) return;
    this.update({ library: null, library_selection: null });
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
  configureDirectory = () => this.beginLibrary("configure");
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
    if (this.libraryTask) return;
    // A read started before an import acknowledgement cannot refresh that import.
    ++this.modelsEpoch;
    this.modelsLoaded = false;
    if (this.modelsPromise) await this.modelsPromise;
    await this.loadPage(null);
  };
  unregisterModel = async (modelId: string, generation: string): Promise<boolean> => {
    if (this.closing || this.state.operation) return false;
    const blocker = modelRemovalBlocker(this.state, modelId);
    if (blocker) { this.report(blocker); return false; }
    if (generation !== this.state.models.generation) {
      this.report(new DesktopError("model_list_changed", "模型列表已变化，请重新打开移除确认。")); return false;
    }
    const model = this.state.models.data.find((entry) => entry.id === modelId)!;
    let removed = false;
    await this.action("正在从模型库移除", async () => {
      this.modelRemovalPending = true;
      ++this.modelsEpoch;
      ++this.modelFeedbackEpoch;
      this.modelConfigurationReads.set(modelId, (this.modelConfigurationReads.get(modelId) ?? 0) + 1);
      let failure: unknown;
      try {
        const result = await this.api.unregisterModel(modelId, generation);
        if (!result || result.model_id !== modelId || result.removed !== true || result.files_preserved !== true)
          throw new DesktopError("model_unregister_unconfirmed", "移除结果尚未确认，请刷新模型列表核对；未自动重试。" );
        // Fence pre-ACK reads before publishing the confirmed removal.
        ++this.snapshotEpoch;
        ++this.modelsEpoch;
        this.modelConfigurationReads.set(modelId, (this.modelConfigurationReads.get(modelId) ?? 0) + 1);
        const model_tests = { ...this.state.model_tests }; delete model_tests[modelId];
        const model_configurations = { ...this.state.model_configurations }; delete model_configurations[modelId];
        const model_configuration_errors = { ...this.state.model_configuration_errors }; delete model_configuration_errors[modelId];
        const snapshot = this.state.snapshot;
        const runtime = snapshot?.runtime;
        this.update({
          models: { ...this.state.models, data: this.state.models.data.filter((entry) => entry.id !== modelId), next_after: null, generation: null },
          page_after: null, model_tests, model_configurations, model_configuration_errors,
          model_removal: { model_id: modelId, id: this.state.operation!.id },
          ...(snapshot && runtime?.selected_model === modelId && ["unloaded", "faulted"].includes(runtime.state)
            ? { snapshot: { ...snapshot, runtime: { ...runtime, selected_model: null, selected_model_display_name: null, load_options: null } } } : {}),
          notice: `已从模型库移除“${model.display_name}”，模型文件已保留。`,
        });
        removed = true;
      } catch (error) {
        const safe = safeError(error);
        failure = ["model_unregister_unconfirmed", "model_list_changed", "model_not_found", "model_unregister_loaded", "runtime_busy", "runtime_running", "invalid_request"].includes(safe.code)
          ? safe : { ...safe, message: "移除结果尚未确认，请刷新模型列表核对；未自动重试。" };
        // Preserve the last observed rows without treating them as current authority.
        // Even a rejection can race another client's mutation before this reread.
        this.update({ models: { ...this.state.models, next_after: null, generation: null }, page_after: null });
      } finally {
        ++this.modelsEpoch;
        ++this.snapshotEpoch;
        this.modelsLoaded = false;
        if (this.modelsPromise) await this.modelsPromise;
        this.modelRemovalPending = false;
      }
      await this.refreshAfterMutation();
      if (!this.modelsLoaded) await this.loadPage(null);
      if (failure) throw failure;
    }, false, false, "remove_model");
    return removed;
  };
  private async beginLibrary(kind: "apply" | "scan" | "discover" | "reconcile" | "add" | "configure") {
    if (this.closing || this.libraryTask || this.downloadTask || this.state.operation || this.stream) return;
    if (kind !== "reconcile" && this.state.snapshot?.connection !== "stopped") {
      this.report(
        new DesktopError(
          "runtime_running",
          kind === "add" ? "请先显式停止运行服务，再添加所选模型。仅卸载模型不够。" : "请先显式停止运行服务，再保存默认目录或手动扫描。仅卸载模型不够。",
        ),
      );
      return;
    }
    const selection = this.state.directory_selection;
    const files = kind === "add" ? this.state.model_selection : null;
    if (kind === "add" && !files) return;
    if ((kind === "apply" || kind === "configure") && !selection) return;
    if (kind === "scan" && !this.state.snapshot?.model_directory.configured) {
      this.report(
        new DesktopError(
          "model_directory_required",
          "请先设置默认下载目录。",
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
      library_task_id: `${this.state.activity_session_id}:library:${++this.nextTask}`,
      library_phase: "starting",
      error: null,
      notice: null,
      library_kind: kind === "add" ? "add" : kind === "configure" ? "configure" : "maintenance",
      library_selection: files,
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
          this.finishUnsubmittedTask("library");
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
      } else handle = kind === "configure"
          ? await this.api.configureDirectory(selection!.selection_id)
          : kind === "add"
          ? await this.api.addModels(files!.selection_id, this.state.add_auto_test && files!.files.length === 1)
          : kind === "discover"
          ? await this.api.discoverDirectory()
          : kind === "apply"
          ? await this.api.applyDirectory(selection!.selection_id)
          : await this.api.scanModels();
      if (kind === "discover" && !handle) {
        this.libraryTask = null;
        ++this.snapshotEpoch;
        this.finishUnsubmittedTask("library");
        this.update({ library_phase: "idle", discovery: "none" });
        return;
      }
      if (!handle?.operation_id)
        throw new DesktopError(
          "invalid_library_operation",
          "模型库操作未返回有效标识。",
        );
      task.id = handle.operation_id;
      if (kind === "apply" || kind === "configure") this.update({ directory_selection: null });
      if (kind === "add") this.update({ model_selection: null, add_auto_test: false });
      this.update({ library_phase: task.cancel ? "stopping" : "running" });
      if (task.cancel) void this.sendLibraryCancel();
      void this.consumeLibrary();
    } catch (error) {
      this.libraryTask = null;
      ++this.snapshotEpoch;
      this.finishUnsubmittedTask("library", safeError(error));
      this.update({ library_phase: "idle", ...(kind === "discover" ? { discovery: "failed" as const } : {}), ...(kind === "reconcile" ? { reconcile_status: "failed" as const } : {}) });
      if (kind === "add" && ["selection_expired", "model_selection_expired", "model_selection_invalid", "model_selection_consumed"].includes(safeError(error).code))
        this.update({ model_selection: null, add_auto_test: false });
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
          !(this.state.library_kind === "add" && this.state.library_selection
            ? validAddOperation(progress, this.state.library_selection)
            : validLibraryOperation(progress, this.state.library_kind === "configure")) || progress.operation_id !== task.id ||
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
          if (this.state.library_kind === "maintenance" && this.state.download?.status === "completed" && progress.result?.directory_id === this.state.download.directory_id)
            this.update({ download: null });
          ++this.modelsEpoch;
          this.modelsLoaded = false;
          this.update({
            models: { data: [], next_after: null, generation: null },
            page_after: null,
            notice: this.state.library_kind === "configure" ? "默认下载目录已保存，原有模型索引保留；未扫描新目录。" : this.state.library_kind === "add" ? null : progress.status === "partial" ? null : progress.result!.registered_files === 0
              ? "默认目录扫描已结束，本次没有登记新的候选；已显式添加的模型保留。"
              : `默认目录扫描完成，本次登记 ${progress.result!.registered_files} 个候选，其中 ${progress.result!.available_files} 个可尝试加载（不代表已实测）。已显式添加的模型保留，本机测试结果以模型列表记录为准。`,
          });
        } else if (progress.status === "cancelled")
          this.update({ notice: this.state.library_kind === "add" ? "添加已取消，已登记结果以逐文件记录为准；源文件未改动。" : "模型库操作已取消，原目录与索引保持不变。" });
        else this.report(progress.error);
        this.update({
          library_phase: "idle",
          operation: { id: ++this.nextOperation, kind: "read_models", label: "正在重新读取模型目录" },
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
    this.update({ download: null, download_task_id: `${this.state.activity_session_id}:download:${++this.nextTask}`, download_phase: "starting", download_auto_test: auto_test === true, error: null, notice: null });
    try {
      const handle = auto_test === undefined ? await this.api.downloadStart(catalog_id) : await this.api.downloadStart(catalog_id, auto_test);
      if (!handle.operation_id) throw new DesktopError("invalid_download_operation", "下载未返回有效标识，未自动重试。");
      task.id = handle.operation_id;
      this.update({ download_phase: task.cancel ? "stopping" : "running" });
      if (task.cancel) void this.sendDownloadCancel();
      void this.consumeDownload();
    } catch (error) {
      this.downloadTask = null;
      this.finishUnsubmittedTask("download", safeError(error));
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
      if (this.downloadTask === task) { task.cancelSent = false; if (this.state.download_phase !== "recovery") this.update({ download_phase: "running" }); this.report(error); }
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
              ? "模型文件已保存，但自动登记未完成；请勿重复下载，可检查登记诊断后用“添加模型”选择已保存文件。"
              : "模型文件已保存，尚未登记。请用“添加模型”选择已保存文件。"}${value.result?.cleanup_warning ? "部分下载文件清理未确认，请勿重复下载。" : ""}` });
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
    ++this.snapshotEpoch;
    if (this.state.snapshot)
      this.update({ snapshot: { ...this.state.snapshot, runtime } });
  }
  /** Native tasks and their outcomes are owned by the controller, never by a route. */
  leaveModelPage = () => {};
  private async awaitCloseDecision() {
    while (this.closeDecision) await this.closeDecision;
  }
  private testScopeCurrent(scope: ModelTestScope): boolean {
    const model = this.state.models.data.find((entry) => entry.id === scope.attempt.model_id);
    if (this.closing || scope.epoch !== this.modelFeedbackEpoch || (model && modelSignature(model) !== scope.attempt.model_signature) ||
        loadOptions(this.state.snapshot?.settings ?? DEFAULT_SETTINGS) !== scope.settings) return false;
    const runtime = this.state.snapshot?.runtime;
    if (scope.backend !== null && runtime && runtime.configured_backend !== scope.backend) return false;
    return scope.attempt.mode !== "test" || this.state.snapshot?.connection === "error" ||
      (this.state.snapshot?.connection === scope.connection && (runtime?.selected_model ?? null) === scope.selected_model &&
        (runtime?.load_options ? loadOptions(runtime.load_options) : null) === scope.runtime_options);
  }
  private finishModelTest(scope: ModelTestScope, result: LocalValidation | null, error: SafeError | null = null, cancelled = false) {
    if (!this.testScopeCurrent(scope)) return;
    this.update({ model_tests: { ...Object.fromEntries(Object.entries(this.state.model_tests).slice(-31)), [scope.attempt.model_id]: {
      ...scope.attempt, phase: "finished", finished_at: Date.now(), result, error, ...(cancelled ? { outcome: "cancelled" as const } : {}),
    } } });
    if (error) { this.report(error); return; }
    if (cancelled) {
      const runtime = this.state.snapshot?.connection === "connected" ? this.state.snapshot.runtime : null;
      this.update({ notice: runtime?.selected_model === scope.attempt.model_id && ["ready", "generating"].includes(runtime.state)
        ? "本次操作已停止，模型仍已加载；本次未取得基础测试通过证明。"
        : "本次加载已停止；当前驻留状态以服务确认结果为准。" });
      return;
    }
    const evidence = this.state.models.data.find((entry) => entry.id === scope.attempt.model_id)?.local_validation;
    this.update({ notice: result ? `${localValidationLabel(result)}${result.state !== "passed" && evidence?.state === "passed" ? "；历史本机通过记录仍保留，不代表本次通过。" : ""}` : "本次操作已结束，尚未取得可确认的短文本测试结果。" });
  }
  private async refreshModelEvidence(scope: ModelTestScope): Promise<SafeError | null> {
    await this.awaitCloseDecision();
    if (!this.testScopeCurrent(scope)) return null;
    ++this.modelsEpoch;
    this.modelsLoaded = false;
    if (this.modelsPromise) await this.modelsPromise;
    if (this.snapshotPromise) await this.snapshotPromise;
    await this.awaitCloseDecision();
    if (!this.testScopeCurrent(scope)) return null;
    await this.refresh(this.state.page_after);
    if (this.snapshotReadError) return { code: "validation_refresh_failed", message: validationErrorReason("validation_refresh_failed") };
    if (!this.modelsLoaded) await this.loadPage(this.state.page_after);
    if (this.modelsReadError || !this.modelsLoaded)
      return { code: "validation_record_read_failed", message: validationErrorReason("validation_record_read_failed") };
    const evidence = this.state.models.data.find((entry) => entry.id === scope.attempt.model_id)?.local_validation;
    if (evidence?.state === "unavailable" || evidence?.error_code?.startsWith("validation_")) {
      const code = evidence.error_code ?? "validation_record_read_failed";
      return { code, message: validationErrorReason(code) };
    }
    if (evidence?.state === "stale") return { code: "validation_scope_changed", message: validationErrorReason("validation_scope_changed") };
    return null;
  }
  private updateModelLoad(task: ModelLoadTask, patch: Partial<ModelLoadTaskView>) {
    if (this.modelLoadTask !== task) return;
    task.view = { ...task.view, ...patch };
    this.update({ model_load: task.view });
  }
  cancelModelLoad = async () => {
    const task = this.modelLoadTask;
    if (!task || task.view.progress?.terminal || task.cancelPending || task.cancelSent) return;
    task.cancel = true;
    if (task.view.cancel_error && this.state.error?.message === task.view.cancel_error.message) this.update({ error: null });
    this.updateModelLoad(task, { phase: task.view.phase === "recovery" ? "recovery" : "stopping", cancel_error: null, stop_requested: true });
    await this.sendModelLoadCancel(task);
    if (task.resume && !task.view.cancel_error) this.recoverModelLoad();
  };
  private async sendModelLoadCancel(task: ModelLoadTask) {
    if (this.modelLoadTask !== task || !task.startSettled || !task.view.operation_id || task.cancelSent || task.cancelPending || !this.api.modelLoadCancel) return;
    task.cancelPending = true;
    try {
      const result = await this.api.modelLoadCancel(task.view.operation_id);
      if (!result || typeof result.stopping !== "boolean") throw new DesktopError("invalid_model_load_operation", "停止请求回执无效，请核对本次任务状态。");
      if (this.modelLoadTask !== task || task.view.progress?.terminal) return;
      task.cancelSent = true;
      task.accepted = true;
      // An acknowledgement is never a terminal result, including stopping:false.
    } catch (error) {
      if (this.modelLoadTask !== task || task.view.progress?.terminal) return;
      const failure = { code: safeError(error).code, message: "停止请求尚未确认，任务仍可能进行。可重试停止或等待真实结果。" };
      this.updateModelLoad(task, { phase: task.view.phase === "recovery" ? "recovery" : "running", cancel_error: failure });
      this.report(failure);
    } finally { task.cancelPending = false; }
  }
  recoverModelLoad = () => {
    const task = this.modelLoadTask;
    if (!task?.resume) return;
    const resume = task.resume;
    task.resume = null;
    this.updateModelLoad(task, { phase: task.cancel && !task.view.cancel_error ? "stopping" : "running" });
    this.update({ error: null });
    resume();
  };
  private async consumeModelLoad(task: ModelLoadTask, scope: ModelTestScope, startError: SafeError | null): Promise<ModelLoadOperation> {
    for (;;) {
      try {
        const progress = await this.api.modelLoadNext!(task.view.operation_id!);
        await this.awaitCloseDecision();
        if (this.modelLoadTask !== task) throw new DesktopError("validation_scope_changed", validationErrorReason("validation_scope_changed"));
        if (!validModelLoadOperation(progress, task.view.operation_id!, task.view.model_id))
          throw new DesktopError("invalid_model_load_operation", "本次加载状态不完整或不匹配，尚未确认任务结束。");
        task.accepted = true;
        if (progress.status === "cancelling") {
          task.cancelSent = true;
          if (task.view.cancel_error && this.state.error?.message === task.view.cancel_error.message) this.update({ error: null });
          this.updateModelLoad(task, { cancel_error: null, stop_requested: true });
        }
        this.updateModelLoad(task, { progress, phase: progress.status === "cancelling" || (task.cancel && !task.view.cancel_error) ? "stopping" : "running" });
        if (progress.runtime && this.testScopeCurrent(scope)) this.setRuntime(progress.runtime);
        if (progress.terminal) {
          if (task.view.cancel_error && this.state.error?.message === task.view.cancel_error.message) this.update({ error: null });
          return progress;
        }
        if (task.cancel && !task.view.cancel_error) void this.sendModelLoadCancel(task);
      } catch (error) {
        if (this.modelLoadTask !== task) throw error;
        // Only the bridge's exact non-ownership reply proves a failed start was not admitted.
        if (!task.accepted && safeError(error).code === "request_not_owned") throw startError ?? error;
        const failure = { code: safeError(error).code, message: "本次加载状态读取失败，尚未确认停止或完成。请重新确认任务状态；不会重新加载。" };
        // Keep the original action and its busy lock until this exact task reaches a terminal.
        await new Promise<void>((resolve) => {
          task.resume = resolve;
          this.updateModelLoad(task, { phase: "recovery" });
          this.report(failure);
        });
      }
      await wait(250);
    }
  }
  private runModelTest = async (modelId: string, mode: "load" | "test", temporary?: Partial<LoadOptions>) => {
    if (this.closing) return;
    if (!this.state.models.generation) {
      this.report(new DesktopError("model_list_changed", "模型列表待确认，请刷新后再操作。")); return;
    }
    const model = this.state.models.data.find((entry) => entry.id === modelId);
    if (!model) { this.report(new DesktopError("model_not_ready", validationErrorReason("model_not_ready"))); return; }
    const snapshot = this.state.snapshot;
    const runtime = snapshot?.connection === "connected" ? snapshot.runtime : null;
    const settings = snapshot?.settings ?? DEFAULT_SETTINGS;
    if (snapshot?.configuration_error) { this.report(snapshot.configuration_error); return; }
    if (mode === "load" && snapshot?.configuration?.schema_version === 1) {
      this.report(new DesktopError("configuration_migration_required", "请先在设置中确认旧配置来源，再按统一档案加载。")); return;
    }
    if (mode === "load" && snapshot?.configuration && !this.api.loadModelProfile && !this.api.loadModelProfileStart) {
      this.report(new DesktopError("configuration_unavailable", "当前桥接未提供统一档案加载能力，请更新应用。")); return;
    }
    if (!snapshot || !["connected", "stopped"].includes(snapshot.connection)) {
      this.report(new DesktopError("runtime_unavailable", "服务状态待确认，请先重新检查服务。")); return;
    }
    const options = mode === "test" && runtime?.load_options ? { ...runtime.load_options } : {
      context_size: settings.context_size, threads: settings.threads, batch_size: settings.batch_size, ...temporary,
    };
    const attempt: ModelTestAttempt = {
      id: ++this.nextTest, model_id: modelId, model_signature: modelSignature(model), mode,
      phase: "running", started_at: Date.now(), finished_at: null, result: null, error: null,
    };
    const scope: ModelTestScope = { attempt, epoch: this.modelFeedbackEpoch, page_after: this.state.page_after,
      options, settings: loadOptions(settings), backend: runtime?.configured_backend ?? null,
      connection: snapshot?.connection ?? null, selected_model: runtime?.selected_model ?? null,
      runtime_options: runtime?.load_options ? loadOptions(runtime.load_options) : null };
    if (this.state.operation || this.stream || this.libraryTask || this.downloadTask || runtime?.stopping ||
        runtime?.registry_busy || runtime?.active_request || (runtime?.queued_jobs ?? 0) > 0 ||
        ["loading", "generating", "unloading"].includes(runtime?.state ?? "")) {
      if (ownRecord(this.state.model_tests, modelId)?.phase === "running") {
        this.update({ notice: "此模型的本次加载或测试仍在进行，请等待本行结果；未重复启动。" });
      } else {
        this.finishModelTest(scope, { state: "deferred", load_success: false, generation_pass: false,
          checked_at_unix_ms: Date.now(), error_code: "runtime_busy" });
      }
      return;
    }
    await this.action(mode === "load" ? "正在加载并进行本机基础测试" : "正在进行本机短文本测试", async () => {
      this.update({ testing_model: modelId, model_tests: { ...this.state.model_tests, [modelId]: attempt } });
      let result: LocalValidation | null = null;
      let failure: SafeError | null = null;
      let cancelled = false;
      const supportsLoadTask = mode === "load" && !!this.api.modelLoadNext && !!this.api.modelLoadCancel &&
        (snapshot?.configuration ? !!this.api.loadModelProfileStart : !!this.api.loadModelStart);
      const task: ModelLoadTask | null = supportsLoadTask ? { view: { attempt_id: attempt.id, model_id: modelId,
        operation_id: crypto.randomUUID(), phase: "starting", progress: null, cancel_error: null, stop_requested: false }, cancel: false, cancelSent: false, cancelPending: false, accepted: false, startSettled: false, resume: null } : null;
      if (task) { this.modelLoadTask = task; this.update({ model_load: task.view }); }
      try {
        if (mode === "load") {
          if (!model.available || model.loadable !== true)
            throw new DesktopError("model_not_ready", "当前模型不可加载，请刷新匹配版本的模型列表。");
          if (snapshot?.connection === "stopped") {
            const started = await this.api.start(!snapshot.initialized);
            // Await the actual close decision; a rejected close does not cancel this operation.
            await this.awaitCloseDecision();
            if (!this.testScopeCurrent(scope)) return;
            this.update({ snapshot: started });
          }
          if (!this.testScopeCurrent(scope)) return;
          if (task) {
            if (task.cancel) cancelled = true; // Startup finished; no load was ever submitted.
            else {
              let startError: SafeError | null = null;
              try {
                const handle = snapshot?.configuration
                  ? await (temporary && Object.keys(temporary).length ? this.api.loadModelProfileStart!(task.view.operation_id!, modelId, temporary) : this.api.loadModelProfileStart!(task.view.operation_id!, modelId))
                  : await this.api.loadModelStart!(task.view.operation_id!, modelId, options);
                if (!handle || handle.operation_id !== task.view.operation_id)
                  throw new DesktopError("invalid_model_load_operation", "加载回执标识不匹配，正在核对原任务；不会重新加载。");
                task.accepted = true;
              } catch (error) { startError = safeError(error); }
              task.startSettled = true;
              this.updateModelLoad(task, { phase: task.cancel ? "stopping" : "running" });
              if (task.cancel) void this.sendModelLoadCancel(task);
              const terminal = await this.consumeModelLoad(task, scope, startError);
              cancelled = terminal.status === "cancelled";
              result = terminal.local_validation;
              if (terminal.status === "failed") failure = { code: terminal.error!.code, message: validationErrorReason(terminal.error!.code) };
              if (!result && !cancelled && !failure && terminal.runtime) {
                result = { state: "loaded", load_success: terminal.runtime.selected_model === modelId && ["ready", "generating"].includes(terminal.runtime.state),
                  generation_pass: false, checked_at_unix_ms: Date.now(), error_code: "validation_result_missing" };
                if (!result.load_success) result = unavailableValidation("validation_result_missing");
              }
            }
          } else {
            const loaded = snapshot?.configuration && this.api.loadModelProfile
              ? await (temporary && Object.keys(temporary).length ? this.api.loadModelProfile(modelId, temporary) : this.api.loadModelProfile(modelId))
              : await this.api.loadModel(modelId, options);
            await this.awaitCloseDecision();
            if (!this.testScopeCurrent(scope)) return;
            this.setRuntime(loaded);
            result = { state: "loaded", load_success: loaded.selected_model === modelId && ["ready", "generating"].includes(loaded.state),
              generation_pass: false, checked_at_unix_ms: Date.now(), error_code: "validation_result_missing" };
            if (!result.load_success) result = unavailableValidation("validation_result_missing");
          }
        } else {
          if (!runtime?.load_options || runtime.selected_model !== modelId || runtime.state !== "ready")
            throw new DesktopError("model_not_ready", validationErrorReason("model_not_ready"));
          result = await this.api.testModel(modelId, options);
          await this.awaitCloseDecision();
          if (!validLocalValidation(result)) {
            result = null;
            throw new DesktopError("invalid_local_validation", validationErrorReason("invalid_local_validation"));
          }
        }
      } catch (error) {
        const code = safeError(error).code;
        failure = { code, message: validationErrorReason(code) };
      }
      try {
        await this.awaitCloseDecision();
        if (!this.testScopeCurrent(scope)) return;
        const readError = await this.refreshModelEvidence(scope);
        if (!failure) failure = readError;
        await this.awaitCloseDecision();
        if (!this.testScopeCurrent(scope)) return;
        if (mode === "load" && !task && !failure && !cancelled) {
          const evidence = this.state.models.data.find((entry) => entry.id === modelId)?.local_validation;
          // model_load returns runtime state, not the probe's DTO. An old Passed is never this attempt's proof.
          if (evidence && evidence.checked_at_unix_ms !== null && evidence.checked_at_unix_ms >= attempt.started_at &&
              evidence.checked_at_unix_ms !== model.local_validation?.checked_at_unix_ms) result = evidence;
        }
        if ((mode === "test" || task) && result?.state === "passed" && !failure) {
          const evidence = this.state.models.data.find((entry) => entry.id === modelId)?.local_validation;
          if (!evidence || evidence.state !== "passed" || evidence.checked_at_unix_ms !== result.checked_at_unix_ms) {
            const code = evidence?.state === "stale" ? "validation_scope_changed" : "validation_result_unconfirmed";
            failure = { code, message: validationErrorReason(code) };
          }
        }
        this.finishModelTest(scope, result, failure, cancelled);
      } finally {
        if (this.modelLoadTask === task) { this.modelLoadTask = null; this.update({ model_load: null }); }
        this.update({ testing_model: null });
      }
    });
    // Early returns while awaiting service startup still release UI ownership.
    if (this.modelLoadTask?.view.attempt_id === attempt.id) { this.modelLoadTask = null; this.update({ model_load: null }); }
    if (this.state.testing_model === modelId) this.update({ testing_model: null });
    const activityId = `${this.state.activity_session_id}:model:${attempt.id}`;
    if (this.state.activities.some((item) => item.id === activityId && ["running", "stopping", "recovery"].includes(item.status))) {
      const stopped: ModelTestAttempt = { ...attempt, phase: "finished", finished_at: Date.now(), result: null, error: { code: "validation_scope_changed", message: validationErrorReason("validation_scope_changed") } };
      const model_tests = { ...this.state.model_tests };
      delete model_tests[modelId];
      this.update({ model_tests, activities: recordActivity(this.state.activities, { id: `${this.state.activity_session_id}:model:${attempt.id}`, kind: "model", label: `模型操作 · ${modelId}`, status: "failed", updated_at: Date.now(), detail: "执行条件已变化或窗口关闭，本次结果不能用于当前组合。", error: stopped.error, model: stopped }) });
    }
  };
  loadModel = (modelId: string, temporary?: Partial<LoadOptions>) => this.runModelTest(modelId, "load", temporary);
  testModel = (modelId: string) => this.runModelTest(modelId, "test");
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
          snapshot: await this.api.saveSettings(preferencesOnly(settings)),
          notice:
            "偏好已保存。加载参数在下次加载时生效，输出预算用于下次发送。",
        });
        if (optionsChanged) {
          this.leaveModelPage();
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
      true, false, "configuration",
    );
  private saveRuntimeSettings(label: string, save: () => Promise<Snapshot>, notice: string) {
    if (this.closing) return Promise.resolve();
    if (this.state.operation || this.stream || this.libraryTask || this.downloadTask) {
      this.update({ notice: null });
      this.report(new DesktopError("operation_in_progress", "有其他操作正在进行，请等待完成或取消后再保存设置。"));
      return Promise.resolve();
    }
    return this.action(label, async () => {
      const epoch = this.settingsEpoch;
      const snapshot = this.state.snapshot;
      if (!snapshot?.initialized) throw new DesktopError("not_initialized", "请先通过左侧服务按钮显式初始化运行服务，再停止服务后保存设置。");
      if (snapshot.connection !== "stopped") throw new DesktopError("runtime_running", "请先显式停止运行服务，再保存设置；不会自动中断任务。");
      try {
        const saved = await save();
        if (epoch !== this.settingsEpoch || this.closing) return;
        this.update({ snapshot: saved, notice, error: null });
      } catch (error) {
        if (epoch !== this.settingsEpoch || this.closing) return;
        const code = safeError(error).code;
        if (code === "runtime_running") throw new DesktopError(code, "运行服务已启动。请先显式停止服务，再保存设置。");
        if (code === "settings_durability_unconfirmed") throw new DesktopError(code, "设置可能已保存，但磁盘持久化尚未确认。请重新读取并核对已保存配置后再重试；当前不代表已回滚。");
        throw error;
      }
    }, false, false, "configuration");
  }
  saveIdle = (seconds: number, enabled?: boolean) =>
    this.saveRuntimeSettings("正在应用空闲卸载设置", async () => {
      const validation = validateIdleSeconds(seconds);
      if (validation) throw new DesktopError("invalid_settings", validation);
      if (enabled !== undefined && (typeof enabled !== "boolean" || typeof this.state.snapshot?.settings.idle_unload_enabled !== "boolean"))
        throw new DesktopError("idle_settings_unavailable", "当前桌面版本未提供不自动卸载设置，请更新桌面应用。");
      return enabled === undefined ? this.api.saveIdle(seconds) : this.api.saveIdle(seconds, enabled);
    }, "空闲卸载设置已保存，下次显式启动运行服务时生效。保存不会启动服务；关闭自动卸载不提供关机或睡眠保护。");
  saveVerificationTimeout = (seconds: number) =>
    this.saveRuntimeSettings("正在保存模型文件校验超时", async () => {
      if (typeof this.state.snapshot?.settings.model_verification_timeout_seconds !== "number")
        throw new DesktopError("verification_settings_unavailable", "当前桌面版本未提供模型文件校验超时设置，请更新桌面应用。");
      const validation = validateVerificationSeconds(seconds);
      if (validation) throw new DesktopError("invalid_settings", validation);
      return this.api.saveVerificationTimeout(seconds);
    }, "模型文件校验超时已保存，后续新校验使用此值；加载前校验在下次显式启动运行服务后使用新值。正在执行的操作不会改动计时。");
  copyToken = () =>
    this.action(
      "正在复制令牌",
      async () => {
        try {
          const copied = await this.api.copyToken();
          if (copied?.copied !== true) throw new Error("copy_unconfirmed");
        } catch (error) {
          const code = safeError(error).code;
          throw new DesktopError(code === "clipboard_unavailable" ? code : "token_copy_failed", "未能确认本机令牌已复制，请检查服务与系统剪贴板后重试。");
        }
        this.update({
          notice:
            "令牌已复制到系统剪贴板。使用后请及时清除剪贴板，避免与他人分享。",
        });
      },
      true,
    );
  refreshLanAddresses = (): Promise<void> => {
    if (this.lanAddressesPromise) return this.lanAddressesPromise;
    this.update({ lan_addresses_loading: true, lan_addresses_error: null });
    this.lanAddressesPromise = Promise.resolve().then(async () => {
      try {
        const result = await this.api.lanAddresses();
        if (!validLanAddresses(result)) throw new DesktopError("lan_address_discovery_invalid", "检测结果无效。");
        this.update({ lan_addresses: result });
      } catch (error) {
        const code = safeError(error).code;
        this.update({ lan_addresses: null, lan_addresses_error: {
          code: /^lan_address_discovery_(failed|busy|timeout|invalid|limit)$/.test(code) ? code : "lan_address_discovery_failed",
          message: "未能检测本机局域网地址。可以刷新重试，或手动填写此电脑的私有 IPv4。",
        } });
      } finally {
        this.lanAddressesPromise = null;
        this.update({ lan_addresses_loading: false });
      }
    });
    return this.lanAddressesPromise;
  };
  saveLanSettings = (lan_api: LanApiSettings) =>
    this.action("正在保存局域网 API 设置", async () => {
      const snapshot = this.state.snapshot;
      if (!snapshot?.lan_api) throw new DesktopError("lan_settings_unavailable", "当前桌面版本未提供局域网 API 设置。");
      if (!snapshot.initialized) throw new DesktopError("not_initialized", "请先显式初始化运行服务，再停止服务后配置局域网 API。");
      if (snapshot.connection !== "stopped") throw new DesktopError("runtime_running", "请先显式停止运行服务，再保存局域网 API 设置；不会自动中断客户端。");
      const validation = validateLanSettings(lan_api);
      if (validation) throw new DesktopError("lan_settings_invalid", validation);
      let saved: Snapshot;
      try { saved = await this.api.saveLanSettings(lan_api); }
      catch (error) {
        const code = safeError(error).code;
        if (code === "runtime_running") throw new DesktopError(code, "运行服务已启动。请先显式停止服务，再保存局域网 API 设置。");
        if (code === "not_initialized") throw new DesktopError(code, "请先显式初始化运行服务，再停止服务后配置局域网 API。");
        if (code === "settings_durability_unconfirmed") throw new DesktopError(code, "局域网设置可能已保存，但磁盘持久化尚未确认。请刷新并核对已保存配置后再重试，当前不代表已回滚。");
        throw error;
      }
      // Invalidate pre-acknowledgement reads before publishing the saved snapshot.
      // action()'s finally runs in a later microtask, after a queued read may settle.
      ++this.snapshotEpoch;
      this.update({
        snapshot: saved,
        notice: lan_api.enabled
          ? "局域网 API 设置已保存，下次显式启动运行服务时生效。保存不会启动服务或生成密钥。"
          : "局域网 API 已配置为关闭。下次启动仅提供本机 API。",
      });
    }, false, false, "configuration");
  copyLanToken = () =>
    this.action("正在复制局域网 API 密钥", async () => {
      if (!this.state.snapshot?.initialized || !this.state.snapshot.lan_api?.enabled)
        throw new DesktopError("lan_token_unavailable", "须先初始化并保存启用局域网 API，再显式启动一次服务以生成独立密钥。");
      try {
        const result = await this.api.copyLanToken();
        if (result?.copied !== true) throw new Error("copy_not_confirmed");
      } catch (error) {
        // Even a malformed native failure must not copy credential-like text into ViewState.
        const code = safeError(error).code;
        if (code === "clipboard_unavailable") throw new DesktopError(code, "无法写入系统剪贴板，局域网密钥未复制。请检查剪贴板后重试。");
        if (code === "preview_only") throw new DesktopError(code, "开发预览不生成或复制真实局域网密钥，请在原生桌面应用中操作。");
        throw new DesktopError("lan_token_unavailable", "未能复制局域网 API 密钥。请确认已保存启用并显式启动过服务；密钥缺失或失效时请检查服务状态后重试。");
      }
      this.update({ notice: "局域网 API 独立密钥已复制到系统剪贴板。仅交给白名单内可信客户端，使用后请及时清除剪贴板。" });
    }, true);
  copyLanBaseUrl = () =>
    this.action("正在复制局域网 Base URL", async () => {
      const settings = this.state.snapshot?.configuration?.saved.lan_api ?? this.state.snapshot?.lan_api;
      const url = settings && lanBaseUrl(settings);
      if (!url) throw new DesktopError("lan_settings_invalid", "请先保存有效的局域网 API 配置。");
      try { await navigator.clipboard.writeText(url); }
      catch { throw new DesktopError("clipboard_unavailable", "无法写入剪贴板，请手动复制页面显示的客户端 Base URL。"); }
      this.update({ notice: "局域网客户端 Base URL 已复制。服务实际监听后，白名单内客户端才可连接。" });
    }, true);
  initialize = () => this.action("正在初始化配置", async () => {
    if (this.state.snapshot?.connection !== "stopped" || this.state.snapshot.initialized) return;
    if (!this.api.initialize) throw new DesktopError("configuration_unavailable", "当前桌面版本不支持离线初始化，请更新应用。");
    const snapshot = await this.api.initialize();
    ++this.snapshotEpoch;
    ++this.configurationReadEpoch;
    this.update({ snapshot, notice: "配置已初始化，服务尚未启动。可先完成配置，再显式启动。" });
  }, false, false, "initialize");
  private applyConfiguration(configuration: ConfigurationSnapshot) {
    ++this.snapshotEpoch;
    ++this.configurationReadEpoch;
    for (const [id, epoch] of this.modelConfigurationReads) this.modelConfigurationReads.set(id, epoch + 1);
    if (this.state.snapshot) this.update({ snapshot: { ...this.state.snapshot, configuration } });
  }
  refreshConfiguration = async () => {
    if (!this.api.configurationGet) return;
    const epoch = ++this.configurationReadEpoch;
    try { const result = await this.api.configurationGet(); if (epoch === this.configurationReadEpoch) this.applyConfiguration(result); }
    catch (error) { if (epoch === this.configurationReadEpoch) this.report(error); }
  };
  refreshModelConfiguration = async (modelId: string) => {
    if (!this.api.configurationModelGet || !this.state.snapshot?.configuration || this.modelRemovalPending) return;
    const epoch = (this.modelConfigurationReads.get(modelId) ?? 0) + 1;
    this.modelConfigurationReads.set(modelId, epoch);
    try {
      const config = await this.api.configurationModelGet(modelId);
      if (this.modelConfigurationReads.get(modelId) !== epoch) return;
      if (config.model_id !== modelId) throw new DesktopError("configuration_unavailable", "档案响应与所选模型不匹配，请重新读取。");
      const errors = { ...this.state.model_configuration_errors }; delete errors[modelId];
      this.update({ model_configurations: { ...Object.fromEntries(Object.entries(this.state.model_configurations).slice(-63)), [modelId]: config }, model_configuration_errors: errors });
    } catch (error) {
      if (this.modelConfigurationReads.get(modelId) !== epoch) return;
      this.update({ model_configuration_errors: { ...this.state.model_configuration_errors, [modelId]: safeError(error) } });
    }
  };
  saveConfiguration = async (request: ConfigurationSaveRequest): Promise<boolean> => {
    let success = false;
    await this.action("正在保存统一配置", async () => {
    if (!this.api.configurationSave) throw new DesktopError("configuration_unavailable", "当前版本不支持统一配置，请更新应用。");
    if (!["model_profile", "request_defaults"].includes(request.update.kind) && this.state.snapshot?.connection !== "stopped") throw new DesktopError("runtime_running", "全局配置需先显式停止服务再保存，不会自动中断任务。");
    try {
      this.applyConfiguration(await this.api.configurationSave(request));
      this.update({ notice: request.update.kind === "model_profile" ? "模型运行档案已保存；当前驻留参数未改变。请另行显式重新加载以应用。" : request.update.kind === "request_defaults" ? "请求默认值已保存；只影响后续新请求，不重载当前模型。" : "全局配置已保存；下次显式启动运行服务时采用。" });
      if (request.update.kind === "model_profile") await this.refreshModelConfiguration(request.update.model_id);
      await this.refreshAfterMutation();
      success = this.snapshotReadError === null && (request.update.kind !== "model_profile" || !ownRecord(this.state.model_configuration_errors, request.update.model_id));
      if (!success) return "recovery" as const;
    } catch (error) {
      if (["configuration_conflict", "configuration_durability_unconfirmed", "configuration_restart_required"].includes(safeError(error).code)) {
        await this.refreshConfiguration();
        if (request.update.kind === "model_profile") await this.refreshModelConfiguration(request.update.model_id);
      }
      throw error;
    }
      }, true, false, "configuration");
    return success;
  };
  migrateConfiguration = (request: ConfigurationMigrateRequest) => this.action("正在迁移统一配置", async () => {
    if (!this.api.configurationMigrate) throw new DesktopError("configuration_unavailable", "当前版本不支持配置迁移。");
    if (this.state.snapshot?.connection !== "stopped") throw new DesktopError("runtime_running", "请先显式停止服务，再选择旧配置来源。");
    try {
      this.applyConfiguration(await this.api.configurationMigrate(request));
      this.update({ notice: "统一配置已迁移。原配置备份由原生端保存，旧版本可能无法读取新格式。" });
      await this.refreshAfterMutation();
      if (this.snapshotReadError) return "recovery" as const;
    } catch (error) { await this.refreshConfiguration(); throw error; }
  }, false, false, "configuration");
  saveUiPreferences = (preferences: UiPreferences, expectedRevision: string) => this.action("正在保存界面偏好", async () => {
    if (!this.api.uiPreferencesSave) throw new DesktopError("configuration_unavailable", "当前版本不支持独立界面偏好。");
    try {
      const saved = await this.api.uiPreferencesSave({ expected_revision: expectedRevision, preferences });
      ++this.snapshotEpoch;
      if (this.state.snapshot) this.update({ snapshot: { ...this.state.snapshot, ui_preferences: saved, settings: { ...this.state.snapshot.settings, ...saved.preferences } }, notice: "界面偏好已保存。" });
    } catch (error) {
      if (this.api.uiPreferencesGet) {
        const saved = await this.api.uiPreferencesGet();
        if (this.state.snapshot) this.update({ snapshot: { ...this.state.snapshot, ui_preferences: saved } });
      }
      throw error;
    }
  }, false, false, "configuration");
  copyPublicText = (value: string, label: string) => this.action(`正在复制${label}`, async () => {
    try { await navigator.clipboard.writeText(value); }
    catch { throw new DesktopError("clipboard_unavailable", "无法写入剪贴板，请手动复制页面显示的内容。"); }
    this.update({ notice: `${label}已复制。` });
  }, true);
  copyLocalBaseUrl = () => {
    const value = localBaseUrl(this.state.snapshot?.api_address);
    if (value) return this.copyPublicText(value, "本机 Base URL");
  };
  copyModelId = (modelId: string) => {
    if (this.state.models.data.some((model) => model.id === modelId) || this.state.snapshot?.runtime?.selected_model === modelId)
      return this.copyPublicText(modelId, "模型 ID");
  };
  private finishUnsubmittedTask(kind: "library" | "download", error: SafeError | null = null) {
    const id = kind === "library" ? this.state.library_task_id : this.state.download_task_id;
    const existing = this.state.activities.find((item) => item.id === id);
    if (!id || !existing) return;
    this.update({ activities: recordActivity(this.state.activities, { ...existing, status: error ? "failed" : "completed", updated_at: Date.now(), detail: error ? "未能确认原生任务已提交，请先核对状态；不会自动重试。" : "检查已结束，没有启动写入任务。", error }) });
  }
  dismissDownloadResult = (result: DownloadOperation) => {
    if (!this.downloadTask && this.state.download_phase === "idle" && this.state.download === result && result.terminal)
      this.update({ download: null });
  };
  restoreActivity = (id: string) => {
    const item = this.state.activities.find((entry) => entry.id === id);
    if (!item || ["running", "stopping", "recovery"].includes(item.status)) return;
    if (item.download && !this.downloadTask) this.update({ download: item.download, download_task_id: item.id });
    if (item.library && !this.libraryTask) this.update({ library: item.library, library_task_id: item.id, library_kind: item.library_kind ?? "maintenance" });
  };
  clearActivityHistory = () => {
    const active = (item: Activity) => !item.id.startsWith("previous:") && ["running", "stopping", "recovery"].includes(item.status);
    const keep = this.state.activities.filter(active);
    const warning = persistActivitySummaries(keep);
    if (warning) { this.update({ activity_storage_warning: warning }, false); return; }
    this.update({ activities: keep, activity_storage_warning: null,
      model_tests: Object.fromEntries(Object.entries(this.state.model_tests).filter(([, value]) => value.phase === "running")),
      ...(this.state.download_phase === "idle" ? { download: null, download_task_id: null } : {}),
      ...(this.state.library_phase === "idle" ? { library: null, library_task_id: null, library_selection: null } : {}),
    }, false);
  };
  recoverStopService = () => this.action("正在检查并停止本机服务", async () => {
    if (!this.state.configuration_recovery_error) return;
    try {
      const result = await this.api.stop();
      if (result?.stopped !== true) throw new DesktopError("runtime_stop_unconfirmed", "清理尚未确认，请重新检查。未自动重试或启动服务。");
    } catch (error) { this.markServiceUnknown(); throw error; }
    await this.refreshAfterMutation();
    this.update({ notice: this.state.configuration_recovery_error ? "停止操作已确认；配置仍不可读取，须修复配置后再启动。" : "停止检查已完成，请以重新读取的当前服务状态为准。" });
  }, false, false, "stop");
  private markServiceUnknown() {
    ++this.snapshotEpoch;
    this.update({ snapshot: this.state.snapshot ? { ...this.state.snapshot, connection: "error", runtime: null, api_address: null } : null });
  }
  checkService = () => this.action("正在检查服务", async () => {
    if (this.snapshotPromise) await this.snapshotPromise;
    await this.refresh();
  }, false, false, "check");
  stop = () =>
    this.action("正在停止运行服务", async () => {
      if (this.state.snapshot?.connection !== "connected" || this.state.snapshot.runtime?.stopping) return;
      try {
        const result = await this.api.stop();
        if (result?.stopped !== true) throw new DesktopError("runtime_stop_unconfirmed", "尚未确认运行服务已停止，请重新检查服务状态。不会自动重试停止。");
      } catch (error) { this.markServiceUnknown(); throw error; }
      ++this.snapshotEpoch;
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
      if (this.snapshotPromise) await this.snapshotPromise;
      await this.refresh();
      if (this.getSnapshot().snapshot?.connection !== "stopped") this.update({ notice: null });
    }, false, false, "stop");
  close = async () => {
    if (this.closing || (this.state.operation && !this.state.testing_model && this.state.operation.kind !== "pick_models")) return;
    this.closing = true;
    let finishDecision!: () => void;
    this.closeDecision = new Promise<void>((resolve) => { finishDecision = resolve; });
    try {
      const selected = !!this.state.model_selection;
      const discarded = this.discardModelSelection();
      if (selected) await discarded;
      await this.api.close();
      // Only a confirmed close invalidates follow-on work. Rejection preserves ownership.
      ++this.modelFeedbackEpoch;
      this.update({ model_tests: {}, activities: this.state.activities.map((item) => item.kind === "model" && ["running", "stopping"].includes(item.status) ? { ...item, status: "recovery" as const, detail: "窗口关闭已确认，本次模型结果未在窗口内完成核对。重开后检查状态，不自动重放。" } : item) });
    }
    catch (error) { this.report(error); }
    finally { this.closing = false; this.closeDecision = null; finishDecision(); }
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
        snapshot.configuration?.runtime_effective?.values.request_defaults.max_output_tokens ?? snapshot.settings.max_output_tokens,
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
      terminal: null as "completed" | "cancelled" | "failed" | null,
      error: null as SafeError | null,
    };
    this.stream = stream;
    this.update({
      activities: recordActivity(this.state.activities, { id: `${this.state.activity_session_id}:chat:${this.nextMessage + 2}`, kind: "chat", label: "辅助聊天测试", status: "running", updated_at: Date.now(), detail: "仅本窗口请求；正文不写入活动记录。", error: null }),
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
      stream.error = safeError(error);
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
    const response = this.state.messages.at(-1);
    const cancelled = this.stream?.terminal === "cancelled";
    if (response) this.update({ activities: recordActivity(this.state.activities, { id: `${this.state.activity_session_id}:chat:${response.id}`, kind: "chat", label: "辅助聊天测试", status: response.state === "complete" ? "completed" : cancelled ? "cancelled" : "failed", updated_at: Date.now(), detail: response.state === "complete" ? "请求已完成；正文仅保留在辅助测试会话。" : "请求已结束；不完整回复不会自动重放。", error: this.stream?.error ?? null }) });
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
      stream.terminal = terminal.type;
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
      if (terminal.type === "failed") { stream.error = safeError(terminal); this.report(terminal); }
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
