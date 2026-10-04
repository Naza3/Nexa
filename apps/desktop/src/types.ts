/** Wire DTOs mirror docs/t06-desktop-contract.md; no token or unrestricted I/O. */
export interface LoadOptions {
  context_size: number;
  threads: number;
  batch_size: number;
}
export type DownloadSource = "modelscope" | "huggingface";
export interface Preferences extends LoadOptions {
  download_source: DownloadSource;
  max_output_tokens: number;
  close_runtime_on_exit: boolean;
}
export interface Settings extends Preferences {
  idle_unload_seconds: number;
}
export interface SafeError {
  code: string;
  message: string;
}
export interface RuntimeStatus {
  state:
    | "unloaded"
    | "loading"
    | "ready"
    | "generating"
    | "unloading"
    | "faulted";
  selected_model: string | null;
  selected_model_display_name: string | null;
  load_options: LoadOptions | null;
  active_request: string | null;
  queued_jobs: number;
  stopping: boolean;
  registry_busy: boolean;
  configured_backend: string;
  backend: string | null;
  backend_observation: string;
  last_error: SafeError | null;
  threads_source?: string | null;
  available_parallelism?: number | null;
  threads_exceed_available_parallelism?: boolean | null;
  worker?: {
    pid: number | null;
    sessions_started: number | null;
    sessions_reaped: number | null;
  };
  memory?: {
    api_private_bytes: number | null;
    worker_private_bytes: number | null;
    gpu_bytes: number | null;
    observation: string;
  };
}
export interface Snapshot {
  initialized: boolean;
  connection: "stopped" | "connecting" | "connected" | "error";
  api_address: string | null;
  runtime: RuntimeStatus | null;
  settings: Settings;
  model_directory: ModelDirectory;
}
export type ModelCompatibility =
  | "admitted"
  | "architecture_unsupported"
  | "quantization_unvalidated"
  | "template_unvalidated"
  | "context_unvalidated"
  | "artifact_unvalidated"
  | "unvalidated"
  | "unknown";
export interface ModelSummary {
  /** Local proof is separate from the historical validated matrix. */
  local_validation?: LocalValidation | null;
  /** Absent on older services; never infer detailed support from the filename. */
  compatibility?: ModelCompatibility;
  id: string;
  display_name: string;
  size_bytes: number;
  sha256: string;
  architecture: string;
  quantization: string;
  validated: boolean;
  loadable?: boolean;
  available: boolean;
  context_limit?: number | null;
  context_size: number | null;
  storage: "managed" | "external";
  availability_error: string | null;
}
export interface LocalValidation {
  state: "untested" | "loaded" | "passed" | "failed" | "stale" | "deferred";
  load_success: boolean;
  generation_pass: boolean;
  checked_at_unix_ms: number | null;
  error_code: string | null;
}
export interface ReconcileResult {
  status: "unchanged" | "observing" | "pending" | "started";
  operation_id: string | null;
}
export interface DirectoryIdentity {
  directory_id: string;
  display_path: string;
  library_generation: string;
}
export interface ModelDirectory {
  configured: DirectoryIdentity | null;
  effective: DirectoryIdentity | null;
  state:
    | "default"
    | "ready"
    | "stopped"
    | "stale"
    | "missing"
    | "unavailable"
    | "unsupported";
}
export interface DirectorySelection {
  selection_id: string;
  display_path: string;
}
export interface ModelFileSelection {
  selection_id: string;
  files: { selection_index: number; file_name: string; size_bytes: number }[];
  expires_in_seconds: number;
}
export interface ModelFileResult {
  selection_index: number;
  file_name: string;
  status: "registered" | "already_registered" | "rejected" | "not_committed" | "not_processed";
  model_id?: string | null;
  error_code?: string | null;
  local_validation?: LocalValidation | null;
}
export interface LibraryResult {
  library_generation: string;
  directory_id: string | null;
  registered_files: number;
  available_files: number;
  rejected_files?: number;
}
export interface LibraryFileError {
  file_name: string;
  code: "invalid_manifest" | "unsupported_model" | "unsupported_chat_template";
  message: string;
}
export interface LibraryOperation {
  operation_id: string;
  status: "running" | "completed" | "partial" | "cancelled" | "failed";
  phase: "checking" | "enumerating" | "verifying" | "committing" | "testing" | "finished";
  examined_entries: number;
  candidate_files: number;
  verified_files: number;
  terminal: boolean;
  result: LibraryResult | null;
  error: SafeError | null;
  failed_file_name: string | null;
  file_errors?: LibraryFileError[];
  /** Present for an explicit selected-file addition; older directory operations omit it. */
  files?: ModelFileResult[];
}
export interface ModelPage {
  /** Older bridges omit the inventory origin. Never infer live residency from it. */
  source?: "local" | "runtime";
  data: ModelSummary[];
  next_after: string | null;
  generation: string;
}
export interface WireMessage {
  role: "user" | "assistant" | "system";
  content: string;
}
export interface Usage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}
export type ChatEvent =
  | { type: "started" }
  | { type: "delta"; text: string }
  | { type: "completed"; finish_reason: "stop" | "length"; usage: Usage }
  | { type: "cancelled" }
  | ({ type: "failed" } & SafeError);
export interface ChatBatch {
  request_id: string;
  events: ChatEvent[];
  terminal: boolean;
}
export interface ChatRequest {
  model_id: string;
  messages: WireMessage[];
  max_output_tokens: number;
}
export interface CatalogSource {
  source: DownloadSource;
  repository: string;
  revision: string;
  url: string;
}
export interface CatalogEntry {
  catalog_id: string;
  display_name: string;
  file_name: string;
  architecture: string;
  quantization: string;
  size_bytes: number;
  sha256: string;
  license: string;
  context_hint: number;
  recommendation: string;
  sources: CatalogSource[];
}
export interface DownloadOperation {
  operation_id: string;
  catalog_id: string;
  source: DownloadSource;
  file_name: string;
  directory_id: string;
  target_display_path: string;
  /** Current in-task transfer attempt; omitted by older bridges (defaults to 1). */
  attempt?: number;
  downloaded_bytes: number;
  total_bytes: number | null;
  phase: "connecting" | "downloading" | "verifying" | "committing" | "registering" | "testing" | "finished";
  status: "running" | "completed" | "cancelled" | "failed";
  terminal: boolean;
  result: { saved: true; registered: boolean; file_name: string; cleanup_warning: string | null; registration_error?: SafeError | null; local_validation?: LocalValidation | null } | null;
  error: SafeError | null;
}
export interface DesktopApi {
  catalog(): Promise<{ entries: CatalogEntry[] }>;
  discoverDirectory(): Promise<{ operation_id: string } | null>;
  downloadStart(catalog_id: string, auto_test?: boolean): Promise<{ operation_id: string }>;
  downloadNext(operation_id: string): Promise<DownloadOperation>;
  downloadCancel(operation_id: string): Promise<{ stopping: boolean }>;
  snapshot(): Promise<Snapshot>;
  start(initialize_if_missing: boolean): Promise<Snapshot>;
  pickDirectory(): Promise<DirectorySelection | null>;
  pickModels(): Promise<ModelFileSelection | null>;
  discardModelSelection(selection_id: string): Promise<{ discarded: boolean }>;
  addModels(selection_id: string, auto_test?: boolean): Promise<{ operation_id: string }>;
  configureDirectory(selection_id: string): Promise<{ operation_id: string }>;
  applyDirectory(selection_id: string): Promise<{ operation_id: string }>;
  scanModels(): Promise<{ operation_id: string }>;
  reconcileModels(): Promise<ReconcileResult>;
  libraryNext(operation_id: string): Promise<LibraryOperation>;
  libraryCancel(
    operation_id: string,
  ): Promise<{ operation_id: string; status: "stopping" }>;
  modelsPage(
    after: string | null,
    generation: string | null,
  ): Promise<ModelPage>;
  loadModel(model_id: string, options: LoadOptions): Promise<RuntimeStatus>;
  testModel(model_id: string, options: LoadOptions): Promise<LocalValidation>;
  unloadModel(): Promise<RuntimeStatus>;
  chatStart(request: ChatRequest): Promise<{ request_id: string }>;
  chatNext(request_id: string): Promise<ChatBatch>;
  chatCancel(
    request_id: string,
  ): Promise<{ request_id: string; status: "stopping" }>;
  saveSettings(settings: Preferences): Promise<Snapshot>;
  saveIdle(idle_unload_seconds: number): Promise<Snapshot>;
  copyToken(): Promise<{ copied: true }>;
  stop(): Promise<{ stopped: true }>;
  close(): Promise<void>;
}
