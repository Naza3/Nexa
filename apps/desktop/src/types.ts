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
  /** Missing on older bridges; old configuration defaults to enabled. */
  idle_unload_enabled?: boolean;
  /** Seconds per verification operation, not per file. Missing on older bridges. */
  model_verification_timeout_seconds?: number;
}
export interface SafeError {
  code: string;
  message: string;
}
/** Public configuration only. LAN credentials never cross the JavaScript bridge. */
export interface LanApiSettings {
  enabled: boolean;
  listen: string | null;
  allowed_cidrs: string[];
}
export interface LanAddress {
  interface_index: number;
  interface_name: string;
  address: string;
}
/** Read-only OS observation, never a trust or reachability guarantee. */
export interface LanAddressDiscovery {
  status: "available" | "empty" | "unsupported";
  addresses: LanAddress[];
}
export interface LanApiStatus {
  enabled: boolean;
  listen: string | null;
  running: boolean;
}
export interface RuntimeStatus {
  /** Missing on older services; never infer an active listener from configuration. */
  lan_api?: LanApiStatus;
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
  configuration?: ConfigurationSnapshot | null;
  configuration_error?: SafeError | null;
  ui_preferences?: UiPreferencesSnapshot | null;
  /** Missing on older bridges, which cannot configure LAN access. */
  lan_api?: LanApiSettings;
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
  state: "untested" | "loaded" | "passed" | "failed" | "stale" | "deferred" | "unavailable";
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
  load_phase?: "preparing" | "loading" | "testing" | null;
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
  load_phase?: "preparing" | "loading" | "testing" | null;
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
/** An opaque handle owns only this window's explicit load and its private probe. */
export interface ModelLoadOperation {
  operation_id: string;
  model_id: string;
  phase: "preparing" | "loading" | "testing" | "finished";
  status: "running" | "cancelling" | "completed" | "cancelled" | "failed";
  terminal: boolean;
  runtime: RuntimeStatus | null;
  local_validation: LocalValidation | null;
  error: SafeError | null;
}
export interface DesktopApi {
  initialize?(): Promise<Snapshot>;
  configurationGet?(): Promise<ConfigurationSnapshot>;
  configurationModelGet?(model_id: string): Promise<ModelConfiguration>;
  configurationSave?(request: ConfigurationSaveRequest): Promise<ConfigurationSnapshot>;
  configurationMigrate?(request: ConfigurationMigrateRequest): Promise<ConfigurationSnapshot>;
  loadModelStart?(operation_id: string, model_id: string, options: LoadOptions): Promise<{ operation_id: string }>;
  loadModelProfileStart?(operation_id: string, model_id: string, load_overrides?: Partial<LoadOptions>): Promise<{ operation_id: string }>;
  modelLoadNext?(operation_id: string): Promise<ModelLoadOperation>;
  modelLoadCancel?(operation_id: string): Promise<{ stopping: boolean }>;
  loadModelProfile?(model_id: string, load_overrides?: Partial<LoadOptions>): Promise<RuntimeStatus>;
  uiPreferencesGet?(): Promise<UiPreferencesSnapshot>;
  uiPreferencesSave?(request: { expected_revision: string; preferences: UiPreferences }): Promise<UiPreferencesSnapshot>;
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
  saveIdle(idle_unload_seconds: number, idle_unload_enabled?: boolean): Promise<Snapshot>;
  saveVerificationTimeout(model_verification_timeout_seconds: number): Promise<Snapshot>;
  copyToken(): Promise<{ copied: true }>;
  lanAddresses(): Promise<LanAddressDiscovery>;
  saveLanSettings(lan_api: LanApiSettings): Promise<Snapshot>;
  copyLanToken(): Promise<{ copied: true }>;
  stop(): Promise<{ stopped: true }>;
  close(): Promise<void>;
}

/** Canonical backend configuration. Never derive or persist this in the frontend. */
export interface LoadDefaults { context_size: number; threads: number | null; batch_size: number }
export interface LoadOverrides { context_size: number | null; threads: number | null; batch_size: number | null }
export interface RequestDefaults { max_output_tokens: number; temperature: number; top_p: number }
export interface RuntimePolicies { idle_unload_enabled: boolean; idle_unload_seconds: number; model_verification_timeout_seconds: number }
export interface ConfigurationValues {
  global_defaults: LoadDefaults;
  request_defaults: RequestDefaults;
  runtime: RuntimePolicies;
  local_api: { listen: string };
  lan_api: LanApiSettings;
  model_profiles: { model_id: string; load_overrides: LoadOverrides }[];
}
export interface ConfigurationSnapshot {
  schema_version: 1 | 2;
  revision: string;
  saved: ConfigurationValues;
  runtime_effective: { revision: string; values: ConfigurationValues } | null;
  pending_restart: boolean;
  migration: {
    state: "not_needed" | "legacy_compatible" | "required" | "complete";
    preferences_revision: string | null;
    differences: { field: "context_size" | "threads" | "batch_size" | "max_output_tokens"; api: number | null; desktop: number }[];
    backup_available: boolean;
  };
}
export interface ModelConfiguration {
  configuration_revision: string;
  model_id: string;
  load_overrides: LoadOverrides;
  saved_effective: LoadOptions;
  saved_sources: { context_size: "global" | "profile"; threads: "global" | "profile" | "automatic"; batch_size: "global" | "profile" };
  current_load_options: LoadOptions | null;
  restore_load_options: LoadOptions | null;
  pending_apply: boolean;
  context_limit: number | null;
}
export type ConfigurationUpdate =
  | { kind: "model_profile"; model_id: string; load_overrides: LoadOverrides }
  | { kind: "global_defaults"; global_defaults: LoadDefaults }
  | { kind: "request_defaults"; request_defaults: RequestDefaults }
  | { kind: "runtime"; runtime: RuntimePolicies }
  | { kind: "local_api"; local_api: { listen: string } }
  | { kind: "lan_api"; lan_api: LanApiSettings };
export interface ConfigurationSaveRequest { expected_revision: string; update: ConfigurationUpdate }
export interface ConfigurationMigrateRequest { expected_revision: string; expected_preferences_revision: string | null; choice: "api" | "desktop" | "custom"; custom: { global_defaults: LoadDefaults; request_defaults: RequestDefaults } | null }
export interface UiPreferences { close_runtime_on_exit: boolean; download_source: DownloadSource }
export interface UiPreferencesSnapshot { revision: string; preferences: UiPreferences }
