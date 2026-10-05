/** Optional recovery cache only. These values are never considered saved backend configuration. */
export const DRAFT_STORAGE_KEY = "nexa.unsaved-drafts.v1";
export const DRAFT_LIMIT = 64;
export const DRAFT_BYTES = 32768;
export interface DraftState<T> { source: T; revision: string | null | undefined; draft: T; conflict: boolean }
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const shape = (value: unknown, keys: string[]): value is Record<string, unknown> => object(value) && Object.keys(value).length === keys.length && keys.every((key) => Object.hasOwn(value, key));
const number = (value: unknown) => value === null || typeof value === "number" && (Number.isNaN(value) || Number.isFinite(value) && Math.abs(value) <= 100000000);
const ui = (value: unknown) => shape(value, ["close_runtime_on_exit", "download_source"]) && typeof value.close_runtime_on_exit === "boolean" && typeof value.download_source === "string" && ["modelscope", "huggingface"].includes(value.download_source);
const load = (value: unknown) => shape(value, ["context_size", "threads", "batch_size"]) && Object.values(value).every(number);
export function validDraftValue(key: string, value: unknown): boolean {
  if (key === "global_defaults") return shape(value, ["global_defaults"]) && load(value.global_defaults);
  if (key === "request_defaults") return shape(value, ["max_output_tokens", "temperature", "top_p"]) && Object.values(value).every(number);
  if (key === "ui_preferences") return ui(value);
  if (key === "legacy_preferences") return shape(value, ["context_size", "threads", "batch_size", "max_output_tokens", "close_runtime_on_exit", "download_source"]) && load({ context_size: value.context_size, threads: value.threads, batch_size: value.batch_size }) && number(value.max_output_tokens) && ui({ close_runtime_on_exit: value.close_runtime_on_exit, download_source: value.download_source });
  if (key === "idle_policy") return shape(value, ["enabled", "seconds"]) && typeof value.enabled === "boolean" && number(value.seconds);
  if (key === "verification_policy") return number(value);
  if (key === "local_api") return shape(value, ["listen"]) && typeof value.listen === "string" && /^[0-9.[\]:]{0,64}$/.test(value.listen);
  if (key === "lan_api") return shape(value, ["enabled", "host", "port", "clients"]) && typeof value.enabled === "boolean" && typeof value.host === "string" && /^[0-9.]{0,64}$/.test(value.host) && typeof value.port === "string" && /^\d{0,5}$/.test(value.port) && typeof value.clients === "string" && value.clients.length <= 512 && /^[0-9./\s]*$/.test(value.clients);
  if (/^model_profile:[a-zA-Z0-9_.-]{1,128}$/.test(key)) return load(value);
  if (/^model_temporary:[a-zA-Z0-9_.-]{1,128}$/.test(key)) return object(value) && Object.keys(value).every((field) => ["context_size", "threads", "batch_size"].includes(field)) && Object.values(value).every(number);
  return false;
}
function revive(key: string, value: unknown): unknown {
  if (key === "verification_policy") return value === null ? Number.NaN : value;
  if (!object(value)) return value;
  if (key === "global_defaults") return { global_defaults: revive("global_load", value.global_defaults) };
  const nullable = key.startsWith("model_profile:") ? ["context_size", "threads", "batch_size"] : key === "global_load" ? ["threads"] : [];
  return Object.fromEntries(Object.entries(value).map(([field, entry]) => [field, entry === null && !nullable.includes(field) ? Number.NaN : entry]));
}
export function readUnsavedDrafts(): Map<string, DraftState<unknown>> {
  const raw = localStorage.getItem(DRAFT_STORAGE_KEY);
  if (!raw) return new Map();
  if (new TextEncoder().encode(raw).length > DRAFT_BYTES) throw new Error("draft_cache_invalid");
  const data: unknown = JSON.parse(raw);
  if (!shape(data, ["version", "entries"]) || data.version !== 1 || !Array.isArray(data.entries) || data.entries.length > DRAFT_LIMIT) throw new Error("draft_cache_invalid");
  const entries = new Map<string, DraftState<unknown>>();
  for (const entry of data.entries) {
    if (!shape(entry, ["key", "source", "revision", "draft"]) || typeof entry.key !== "string" || entries.has(entry.key) || (entry.revision !== null && (typeof entry.revision !== "string" || !/^(absent|sha256:[a-f0-9]{64})$/.test(entry.revision))) || !validDraftValue(entry.key, entry.source) || !validDraftValue(entry.key, entry.draft)) throw new Error("draft_cache_invalid");
    entries.set(entry.key, { source: revive(entry.key, entry.source), revision: entry.revision ?? undefined, draft: revive(entry.key, entry.draft), conflict: false });
  }
  return entries;
}
export class DraftStore extends Map<string, DraftState<unknown>> {
  pending = new Map<string, DraftState<unknown>>();
  warning: string | null = null;
  private version = 0;
  private listeners = new Set<() => void>();
  private queued = false;
  constructor() {
    super();
    try { this.pending = readUnsavedDrafts(); }
    catch { this.warning = "未采用无法验证的草稿缓存。当前草稿仅能保证在本窗口保留，关闭可能丢失。"; }
  }
  getSnapshot = () => this.version;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private changed() {
    ++this.version;
    if (this.queued) return;
    this.queued = true;
    queueMicrotask(() => { this.queued = false; this.listeners.forEach((listener) => listener()); });
  }
  override set(key: string, value: DraftState<unknown>) {
    super.set(key, value);
    this.persist();
    this.changed();
    return this;
  }
  private persist(): boolean {
    const recovery = new Map(this.pending);
    let pendingCollision = false;
    for (const [key, value] of this) {
      if (this.pending.has(key)) { pendingCollision ||= !same(value.source, value.draft); continue; }
      if (same(value.source, value.draft)) recovery.delete(key);
      else recovery.set(key, value);
    }
    try {
      if (recovery.size > DRAFT_LIMIT) throw new Error("draft_limit");
      const entries = [...recovery].map(([key, value]) => {
        if (!validDraftValue(key, value.source) || !validDraftValue(key, value.draft)) throw new Error("draft_invalid");
        return { key, source: value.source, revision: value.revision ?? null, draft: value.draft };
      });
      const raw = JSON.stringify({ version: 1, entries });
      if (new TextEncoder().encode(raw).length > DRAFT_BYTES) throw new Error("draft_limit");
      if (entries.length) localStorage.setItem(DRAFT_STORAGE_KEY, raw);
      else localStorage.removeItem(DRAFT_STORAGE_KEY);
      this.warning = pendingCollision ? "同组上次草稿尚未确认，已保留原恢复副本。当前新修改暂仅在本窗口，请先恢复或丢弃上次草稿。" : null;
      return true;
    } catch { this.warning = "草稿恢复缓存未能保存。最新修改仅保留在本窗口；关闭窗口可能丢失，请先保存到后端或保留窗口。"; return false; }
  }
  restorePending() {
    for (const [key, value] of this.pending) {
      const current = this.get(key);
      if (!current || same(current.source, current.draft)) super.set(key, value);
    }
    this.pending.clear();
    this.persist(); this.changed();
  }
  discardPending() {
    const previous = this.pending;
    this.pending = new Map();
    if (!this.persist()) this.pending = previous;
    this.changed();
  }
  /** Used only after explicit discard. A storage failure keeps the window open. */
  discardAll(): boolean {
    try { localStorage.removeItem(DRAFT_STORAGE_KEY); }
    catch { this.warning = "无法清除恢复草稿，尚未关闭窗口。请检查本机存储后重试。"; this.changed(); return false; }
    super.clear(); this.pending.clear(); this.warning = null; this.changed(); return true;
  }
}
export const createDraftStore = () => new DraftStore();
export const hasDirtyDrafts = (store: DraftStore) => store.pending.size > 0 || [...store.values()].some((form) => !same(form.draft, form.source));
