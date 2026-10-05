import { useContext } from "react";
import type { ReactNode } from "react";
import { ConfigDraftContext } from "./configDraft";

/** Keep editors mounted so collapsing a section never discards a draft or CAS warning. */
export function DetailsGroup({ title, description, draftKeys = [], children, initialOpen = false, status }: {
  title: string; description?: string; draftKeys?: string[]; children: ReactNode; initialOpen?: boolean; status?: string;
}) {
  const drafts = useContext(ConfigDraftContext);
  const forms = draftKeys.flatMap((key) => drafts?.get(key) ? [drafts.get(key)!] : []);
  const conflict = forms.some((form) => form.conflict);
  const dirty = forms.some((form) => JSON.stringify(form.draft) !== JSON.stringify(form.source));
  return <details className="details-group" open={initialOpen || undefined}>
    <summary><span><strong>{title}</strong>{description && <small>{description}</small>}</span>{(dirty || conflict || status) && <span className="draft-indicator">{conflict ? "版本变化 · 待核对" : dirty ? "未保存" : status}</span>}</summary>
    <div className="details-group-content">{children}</div>
  </details>;
}
