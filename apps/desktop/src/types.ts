/** Wire DTOs mirror docs/t06-desktop-contract.md; no token or unrestricted I/O. */
export interface LoadOptions {
  context_size: number;
  threads: number;
  batch_size: number;
}
export interface Preferences extends LoadOptions {
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
export interface LibraryResult {
  library_generation: string;
  directory_id: string;
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
  phase: "checking" | "enumerating" | "verifying" | "committing" | "finished";
  examined_entries: number;
  candidate_files: number;
  verified_files: number;
  terminal: boolean;
  result: LibraryResult | null;
  error: SafeError | null;
  failed_file_name: string | null;
  file_errors?: LibraryFileError[];
}
export interface ModelPage {
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
export interface DesktopApi {
  snapshot(): Promise<Snapshot>;
  start(initialize_if_missing: boolean): Promise<Snapshot>;
  pickDirectory(): Promise<DirectorySelection | null>;
  applyDirectory(selection_id: string): Promise<{ operation_id: string }>;
  scanModels(): Promise<{ operation_id: string }>;
  libraryNext(operation_id: string): Promise<LibraryOperation>;
  libraryCancel(
    operation_id: string,
  ): Promise<{ operation_id: string; status: "stopping" }>;
  modelsPage(
    after: string | null,
    generation: string | null,
  ): Promise<ModelPage>;
  loadModel(model_id: string, options: LoadOptions): Promise<RuntimeStatus>;
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
