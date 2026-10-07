import type { ConfigurationSnapshot, ModelConfiguration, Snapshot } from "../src/types";
import { model, snapshot } from "./fixtures";
export const revision = (digit = "a") => `sha256:${digit.repeat(64)}`;
export function configuration(): ConfigurationSnapshot {
  return { schema_version: 2, revision: revision(), saved: {
    global_defaults: { context_size: 4096, threads: null, batch_size: 512 }, request_defaults: { max_output_tokens: 768, temperature: 0.8, top_p: 0.95 },
    runtime: { execution_timeout_seconds: 300, idle_unload_enabled: true, idle_unload_seconds: 300, model_verification_timeout_seconds: 600 }, local_api: { listen: "127.0.0.1:18181" },
    lan_api: { enabled: false, listen: null, allowed_cidrs: [] }, model_profiles: [],
  }, runtime_effective: null, pending_restart: false, migration: { state: "not_needed", preferences_revision: null, differences: [], backup_available: false } };
}
export function configuredSnapshot(stopped = false): Snapshot {
  const value = snapshot();
  value.configuration = configuration();
  value.ui_preferences = { revision: revision("c"), preferences: { download_source: "modelscope", close_runtime_on_exit: false } };
  if (stopped) { value.connection = "stopped"; value.runtime = null; }
  else value.configuration.runtime_effective = { chat_response_timeout_seconds: 1350, revision: revision(), values: structuredClone(value.configuration.saved) };
  return value;
}
export function modelConfiguration(): ModelConfiguration {
  return { model_id: model.id, configuration_revision: revision(), load_overrides: { context_size: null, threads: null, batch_size: null }, saved_effective: { context_size: 4096, threads: 4, batch_size: 512 }, saved_sources: { context_size: "global", threads: "automatic", batch_size: "global" }, current_load_options: { context_size: 2048, threads: 2, batch_size: 128 }, restore_load_options: null, pending_apply: true, context_limit: 40960 };
}
