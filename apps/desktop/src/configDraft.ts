import { createContext, useContext, useState } from "react";
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
import { DraftStore } from "./unsavedDrafts";
import type { DraftState } from "./unsavedDrafts";
export { createDraftStore, hasDirtyDrafts } from "./unsavedDrafts";
export const ConfigDraftContext = createContext<DraftStore | null>(null);
/** Drafts retain the original backend revision. Recovery always requires user confirmation. */
export function useConfigDraft<T>(source: T, revision?: string | null, key?: string) {
  const store = useContext(ConfigDraftContext);
  const [form, setState] = useState<DraftState<T>>(() => key && store?.has(key)
    ? store.get(key) as DraftState<T> : { source, revision, draft: source, conflict: false });
  const setForm = (next: DraftState<T>) => {
    if (key) store?.set(key, next);
    setState(next);
  };
  const changedSource = !same(form.source, source) || form.revision !== revision;
  const dirty = !same(form.draft, form.source);
  if (changedSource && (!dirty || same(form.draft, source))) {
    setForm({ source, revision, draft: source, conflict: false });
  } else if (changedSource && !form.conflict) {
    setForm({ ...form, conflict: true });
  }
  return {
    draft: form.draft,
    base: form.source,
    baseRevision: form.revision,
    dirty,
    conflict: form.conflict || (changedSource && dirty && !same(form.draft, source)),
    setDraft: (draft: T) => setForm({ ...form, draft }),
    reset: () => setForm({ source, revision, draft: source, conflict: false }),
    rebase: () => setForm({ source, revision, draft: form.draft, conflict: false }),
  };
}
