import type { DesktopApi, LoadOptions, ModelConfiguration, WorkbenchPreferences, WorkbenchPreferencesSnapshot } from "./types";

export const OCR_DEFAULT_LOAD: LoadOptions = { context_size: 8192, threads: 4, batch_size: 256 };
export const defaultWorkbench = (): WorkbenchPreferences => ({ close_to_tray: false, ocr: { ...OCR_DEFAULT_LOAD, model_id: null, max_output_tokens: 2048, prompt: "Text Recognition:", image_edge: 0, markdown: false, model_drafts: {} }, chat: { draft: "" } });
const integer = (value: number, min: number, max: number) => Number.isSafeInteger(value) && value >= min && value <= max;
export const validLoad = (value: LoadOptions) => !!value && integer(value.context_size, 32, 131072) && integer(value.threads, 1, 256) && integer(value.batch_size, 1, Math.min(value.context_size, 4096));
export const sameLoad = (a: LoadOptions, b: LoadOptions) => a.context_size === b.context_size && a.threads === b.threads && a.batch_size === b.batch_size;
const invalidInput = "工作区输入尚未保存：请检查参数范围；提示词最多 4096 UTF-8 字节，聊天草稿最多 16 KiB。";
export function validWorkbench(p: WorkbenchPreferences): boolean {
  return typeof p?.close_to_tray === "boolean" && !!p.ocr && !!p.chat && validLoad(p.ocr) && integer(p.ocr.max_output_tokens, 1, 4096) && typeof p.ocr.prompt === "string" && new TextEncoder().encode(p.ocr.prompt).length <= 4096 &&
    [0, 1600, 2048].includes(p.ocr.image_edge) && typeof p.ocr.markdown === "boolean" && (p.ocr.model_id === null || typeof p.ocr.model_id === "string" && !!p.ocr.model_id) &&
    typeof p.chat.draft === "string" && new TextEncoder().encode(p.chat.draft).length <= 16384 && !!p.ocr.model_drafts && typeof p.ocr.model_drafts === "object" && !Array.isArray(p.ocr.model_drafts) && Object.values(p.ocr.model_drafts).every((v) => !!v && validLoad(v.base) && validLoad(v.draft));
}
export function resolveOcrProfile(config: ModelConfiguration, saved?: { base: LoadOptions; draft: LoadOptions }) {
  const base = config.saved_effective;
  if (!saved) return { base, draft: Object.values(config.load_overrides).some((v) => v !== null) ? base : OCR_DEFAULT_LOAD, conflict: false };
  if (sameLoad(saved.draft, saved.base) || sameLoad(saved.draft, base)) return { base, draft: base, conflict: false };
  return { base, draft: saved.draft, conflict: !sameLoad(saved.base, base) };
}
export interface WorkbenchState { preferences: WorkbenchPreferences; revision: string | null; hydrated: boolean; supported: boolean; dirty: boolean; saving: boolean; conflict: boolean; error: string | null }
export const initialWorkbench = (supported = false): WorkbenchState => ({ preferences: defaultWorkbench(), revision: null, hydrated: !supported, supported, dirty: false, saving: false, conflict: false, error: null });

/** One writer shared by chat and OCR. Revisions advance only after a successful CAS. */
export class WorkbenchStore {
  state: WorkbenchState;
  private read: Promise<void> | null = null;
  private write: Promise<boolean> | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private version = 0;
  constructor(private api: DesktopApi, private publish: (state: WorkbenchState) => void) { this.state = initialWorkbench(!!api.workbenchGet && !!api.workbenchSave); }
  private update(patch: Partial<WorkbenchState>) { this.state = { ...this.state, ...patch }; this.publish(this.state); }
  load = (replace = false): Promise<void> => {
    if (!this.state.supported || this.state.hydrated && !replace) return Promise.resolve();
    if (this.read) return this.read;
    if (this.write) return this.write.then(() => this.load(replace));
    if (replace) this.update({ hydrated: false });
    this.read = (async () => {
      try {
        const result = await Promise.resolve().then(() => this.api.workbenchGet!());
        if (!result?.revision || !validWorkbench(result.preferences)) throw new Error();
        this.version++;
        this.update({ preferences: result.preferences, revision: result.revision, hydrated: true, dirty: false, conflict: false, error: null });
      } catch { this.update({ error: "工作区设置读取失败。为避免覆盖已保存内容，读取成功前不会保存默认值。" }); }
      finally { this.read = null; }
    })();
    return this.read;
  };
  edit = (preferences: WorkbenchPreferences) => {
    if (!this.state.hydrated) return;
    if (JSON.stringify(preferences) === JSON.stringify(this.state.preferences)) return;
    this.version++;
    this.update({ preferences, dirty: this.state.supported, error: this.state.conflict ? this.state.error : validWorkbench(preferences) ? null : invalidInput });
    clearTimeout(this.timer);
    if (this.state.supported && !this.state.conflict && validWorkbench(preferences)) this.timer = setTimeout(() => { void this.flush(); }, 400);
  };
  flush = (): Promise<boolean> => {
    clearTimeout(this.timer);
    if (this.write) return this.write;
    if (!this.state.supported || !this.state.dirty) return Promise.resolve(true);
    if (!this.state.hydrated || this.state.conflict) return Promise.resolve(false);
    if (!validWorkbench(this.state.preferences)) { this.update({ error: invalidInput }); return Promise.resolve(false); }
    this.write = (async () => {
      this.update({ saving: true });
      try {
        while (this.state.dirty) {
          if (!validWorkbench(this.state.preferences)) { this.update({ error: invalidInput }); return false; }
          const version = this.version;
          const preferences = structuredClone(this.state.preferences);
          const saved: WorkbenchPreferencesSnapshot = await this.api.workbenchSave!({ expected_revision: this.state.revision!, preferences });
          if (!saved?.revision || !validWorkbench(saved.preferences)) throw new Error();
          this.update({ revision: saved.revision, ...(version === this.version ? { preferences: saved.preferences, dirty: false } : {}), error: null, conflict: false });
        }
        return true;
      } catch (error) {
        const conflict = !!error && typeof error === "object" && "code" in error && String(error.code).includes("conflict");
        this.update({ conflict, error: conflict ? "工作区设置已被其他窗口修改。当前输入未保存，请选择读取已保存设置，或确认保留当前输入后覆盖。" : "工作区设置保存失败，当前输入仍保留。请重试保存后再关闭。" });
        return false;
      } finally { this.update({ saving: false }); this.write = null; }
    })();
    return this.write;
  };
  overwrite = async () => {
    if (!this.state.supported || this.write) return;
    try {
      const latest = await this.api.workbenchGet!();
      if (!latest?.revision || !validWorkbench(latest.preferences)) throw new Error();
      this.update({ revision: latest.revision, conflict: false, error: null });
      await this.flush();
    } catch { this.update({ error: "无法读取最新工作区设置，当前输入尚未保存。" }); }
  };
}
