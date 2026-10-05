import { validOptionalLoadPhase } from "./modelLoad";
import { validLocalValidation } from "./localValidation";
import type { LibraryOperation, ModelFileSelection } from "./types";

const bytes = (value: string) => new TextEncoder().encode(value).byteLength;
const text = (value: unknown, max: number) => typeof value === "string" && value.length > 0 && bytes(value) <= max && !value.includes("\0");
const integer = (value: unknown, max: number) => typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value <= max;
const filename = (value: unknown) => text(value, 1024) && !/[/\\]/.test(value as string);
const code = (value: unknown) => typeof value === "string" && /^[a-z0-9_]{1,80}$/.test(value);

/** Paths stay native. Names and indices only identify rows within an opaque selection. */
export function validModelSelection(value: ModelFileSelection): boolean {
  return !!value && typeof value === "object" && text(value.selection_id, 128) &&
    integer(value.expires_in_seconds, 600) && value.expires_in_seconds > 0 &&
    Array.isArray(value.files) && value.files.length > 0 && value.files.length <= 64 &&
    value.files.every((file, index) => file && file.selection_index === index &&
      filename(file.file_name) && /\.gguf$/i.test(file.file_name) && integer(file.size_bytes, 16 * 1024 ** 3)) &&
    value.files.reduce((total, file) => total + file.size_bytes, 0) <= 32 * 1024 ** 3 &&
    bytes(JSON.stringify(value)) <= 128 * 1024;
}

/** Selected-file operations have additive, per-file results, unlike full directory scans. */
export function validAddOperation(value: LibraryOperation, selection: ModelFileSelection): boolean {
  if (!value || typeof value !== "object" || !validOptionalLoadPhase(value.load_phase) || bytes(JSON.stringify(value)) > 1024 * 1024 ||
    !text(value.operation_id, 128) || !["running", "completed", "partial", "cancelled", "failed"].includes(value.status) ||
    !["checking", "enumerating", "verifying", "committing", "testing", "finished"].includes(value.phase) ||
    !integer(value.examined_entries, selection.files.length) || value.candidate_files !== selection.files.length ||
    !integer(value.verified_files, value.examined_entries) ||
    (value.error !== null && (!value.error || !code(value.error.code) || !text(value.error.message, 500))) ||
    !Array.isArray(value.files) || value.files.length > selection.files.length ||
    value.files.some((file) => !file || typeof file !== "object") ||
    new Set(value.files.map((file) => file.selection_index)).size !== value.files.length) return false;
  for (const file of value.files) {
    const original = selection.files[file.selection_index];
    if (!integer(file.selection_index, selection.files.length - 1) || !original ||
      file.file_name !== original.file_name ||
      !["registered", "already_registered", "rejected", "not_committed", "not_processed"].includes(file.status) ||
      (file.model_id != null && !text(file.model_id, 256)) ||
      (file.error_code != null && !code(file.error_code)) ||
      (file.local_validation != null && !validLocalValidation(file.local_validation))) return false;
    const registered = file.status === "registered" || file.status === "already_registered";
    if ((registered && (!file.model_id || file.error_code != null)) ||
      (!registered && file.local_validation != null) ||
      (registered && value.status === "running" && !["committing", "testing"].includes(value.phase))) return false;
  }
  const durabilityUnconfirmed = value.status === "failed" && value.error?.code === "settings_durability_unconfirmed" && value.result !== null;
  const published = value.status === "completed" || value.status === "partial" || durabilityUnconfirmed;
  const successful = value.files.filter((file) => file.status === "registered" || file.status === "already_registered").length;
  const rejected = value.files.filter((file) => file.status === "rejected").length;
  if (published) {
    const result = value.result;
    if (!result || (!durabilityUnconfirmed && value.error) || !text(result.library_generation, 128) ||
      (result.directory_id !== null && !text(result.directory_id, 128)) ||
      value.files.length !== selection.files.length || successful !== value.verified_files ||
      result.registered_files !== successful || result.rejected_files !== rejected ||
      !integer(result.available_files, successful) || successful + rejected !== selection.files.length ||
      (value.status === "partial" && (!successful || !rejected)) ||
      (value.status === "completed" && rejected !== 0)) return false;
  } else if (value.result !== null || (value.status === "failed" && !value.error) ||
    (value.status !== "running" && successful > 0) || (value.status === "running" && value.error !== null)) return false;
  return value.terminal === (value.status !== "running") && (!value.terminal || value.phase === "finished");
}
