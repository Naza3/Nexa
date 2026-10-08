import { useEffect, useRef, useState } from "react";
import type { DesktopApi, OcrHistoryEntry, OcrHistoryList, OcrHistorySummary } from "./types";
import { ChatMarkdown } from "./ChatMarkdown";
import { PerformanceSummary } from "./PerformanceSummary";
import { validatePerformance } from "./performance";

function validSummary(row: OcrHistorySummary): boolean {
  return !!row && typeof row.id === "string" && !!row.id && typeof row.image_name === "string" && typeof row.model_id === "string" && typeof row.incomplete === "boolean" &&
    ["completed", "cancelled", "failed"].includes(row.status) && [null, "stop", "length"].includes(row.finish_reason) &&
    (row.error_code === null || typeof row.error_code === "string") && Number.isSafeInteger(row.first_saved_at_unix_ms) && row.first_saved_at_unix_ms >= 0 &&
    (row.incomplete || row.status === "completed" && row.finish_reason === "stop") &&
    Number.isSafeInteger(row.markdown_bytes) && row.markdown_bytes > 0 && row.markdown_bytes <= 256 * 1024;
}
function validateList(value: OcrHistoryList): OcrHistoryList {
  if (!value || value.capacity !== 100 || !Array.isArray(value.entries) || value.entries.length > 100 || !value.entries.every(validSummary) || new Set(value.entries.map((row) => row.id)).size !== value.entries.length) throw new Error("invalid_history");
  return value;
}
function validateEntry(value: OcrHistoryEntry, id: string): OcrHistoryEntry {
  if (!value || typeof value.markdown !== "string" || value.id !== id || !validSummary({ ...value, markdown_bytes: new TextEncoder().encode(value.markdown).length })) throw new Error("invalid_history");
  if (value.performance !== null) {
    const snapshot = validatePerformance({ instance_id: value.performance.instance_id, capacity: 200, records: [value.performance.record] });
    const row = snapshot.records[0];
    if (row.request_id !== value.id || row.model_id !== value.model_id || row.modality !== "image" || row.status !== value.status || row.finish_reason !== value.finish_reason || row.error_code !== value.error_code) throw new Error("invalid_history");
  }
  return value;
}
const label = (row: OcrHistorySummary | OcrHistoryEntry) => row.status === "completed" ? row.finish_reason === "length" ? "成功 · 输出达上限" : row.incomplete ? "成功终态 · 正文可能缺失" : "成功" : row.status === "cancelled" ? "已取消 · 部分结果" : "失败 · 部分结果";

export function OcrHistory({ api, changed, removeRecord, closing = false }: { api: DesktopApi; changed: number; removeRecord?: (id: string) => Promise<OcrHistoryList>; closing?: boolean }) {
  const [list, setList] = useState<OcrHistoryList | null>(null);
  const [selected, setSelected] = useState<OcrHistoryEntry | null>(null);
  const [markdown, setMarkdown] = useState(false);
  const [revision, setRevision] = useState(0);
  const [loading, setLoading] = useState(!!api.ocrHistoryList);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [deleting, setDeleting] = useState<string | null>(null);
  const [owner, setOwner] = useState(api);
  if (owner !== api) { setOwner(api); setList(null); setSelected(null); setError(""); setNotice(""); setDeleting(null); setLoading(!!api.ocrHistoryList); }
  const detailEpoch = useRef(0);
  const lifecycle = useRef(0);
  useEffect(() => () => { ++detailEpoch.current; ++lifecycle.current; }, [api]);
  useEffect(() => {
    let alive = true;
    if (api.ocrHistoryList) {
      void api.ocrHistoryList().then(validateList).then((result) => { if (alive) { setList(result); setSelected((entry) => entry && result.entries.some((row) => row.id === entry.id) ? entry : null); setError(""); } }).catch(() => { if (alive) setError("无法读取本机识别历史，请刷新重试。当前识别正文仍保留。"); }).finally(() => { if (alive) setLoading(false); });
    }
    return () => { alive = false; };
  }, [api, changed, revision]);
  const open = async (id: string) => {
    if (!api.ocrHistoryGet) return;
    const epoch = ++detailEpoch.current;
    setSelected(null); setNotice("正在读取历史结果…");
    try {
      const result = validateEntry(await api.ocrHistoryGet(id), id);
      if (epoch === detailEpoch.current) { setSelected(result); setNotice(""); }
    } catch { if (epoch === detailEpoch.current) setNotice("这条历史记录无法读取，可能已删除或被最近 100 条限制淘汰。请刷新列表。"); }
  };
  const remove = async (id: string) => {
    if (!api.ocrHistoryDelete || deleting) return;
    const epoch = lifecycle.current;
    setDeleting(id);
    try {
      await (removeRecord ?? api.ocrHistoryDelete)(id);
      if (epoch !== lifecycle.current) return;
      ++detailEpoch.current;
      setSelected((current) => current?.id === id ? null : current);
      setList((current) => current ? { ...current, entries: current.entries.filter((row) => row.id !== id) } : null);
      setNotice("已删除这条本机识别记录。"); setRevision((value) => value + 1);
    } catch { if (epoch === lifecycle.current) setNotice("删除失败，记录可能仍保留，请刷新后重试。"); }
    finally { if (epoch === lifecycle.current) setDeleting(null); }
  };
  return <details className="ocr-card ocr-history"><summary><strong>最近识别结果</strong><span>本机保存 · 最多 100 条</span></summary><div className="ocr-history-content">
    <p className="small-note">自动保存有正文的识别终态，重启后可查看；超过 100 条自动淘汰最早记录。仅保存结果文字、图片文件名与性能信息，不保存原图。失败或取消的内容可能不完整。</p>
    {!api.ocrHistoryList ? <p role="status">当前桌面版本不支持本机识别历史，请更新应用。当前结果仍可复制或另存。</p> : <>
      <div className="workspace-actions"><button disabled={loading} onClick={() => { setLoading(true); setRevision((value) => value + 1); }}>{loading ? "正在读取历史…" : "刷新识别历史"}</button><span>{list ? `${list.entries.length} / 100 条` : ""}</span></div>
      {error && <p role="status" className="warning-text">{error}</p>}
      {list?.entries.length === 0 && <p>暂无已保存的识别结果。</p>}
      {!!list?.entries.length && <div className="ocr-history-list" role="region" aria-label="识别历史列表" tabIndex={0}><table><thead><tr><th>保存时间</th><th>图片 / 模型</th><th>结果</th><th>操作</th></tr></thead><tbody>{list.entries.map((row) => <tr key={row.id}><td>{new Date(row.first_saved_at_unix_ms).toLocaleString()}</td><td>{row.image_name}<small>{row.model_id}</small></td><td>{label(row)}</td><td><button disabled={!api.ocrHistoryGet} onClick={() => void open(row.id)} aria-label={`查看识别记录 ${row.image_name}`}>查看</button><button disabled={closing || !!deleting || !api.ocrHistoryDelete} onClick={() => void remove(row.id)} aria-label={`删除识别记录 ${row.image_name}`}>{deleting === row.id ? "删除中…" : "删除"}</button></td></tr>)}</tbody></table></div>}
    </>}
    {notice && <p role="status">{notice}</p>}
    {selected && <section className="ocr-history-detail" aria-label="历史识别详情"><h3>{selected.image_name}</h3><p>{label(selected)} · {selected.model_id}{selected.error_code ? ` · ${selected.error_code}` : ""}</p>
      <div className="ocr-result-toolbar"><div className="ocr-view-switch" role="group" aria-label="历史结果显示方式"><button aria-pressed={!markdown} onClick={() => setMarkdown(false)}>历史原文</button><button aria-pressed={markdown} onClick={() => setMarkdown(true)}>历史 Markdown</button></div><div className="ocr-export-actions"><button onClick={() => void navigator.clipboard.writeText(selected.markdown).then(() => setNotice("已复制历史原文。")).catch(() => setNotice("复制失败，请重试。"))}>复制历史原文</button><button disabled={!api.saveOcrMarkdown} onClick={() => void api.saveOcrMarkdown?.(selected.markdown).then((result) => setNotice(result.saved ? "已另存历史 Markdown。" : "已取消另存。")).catch(() => setNotice("另存失败，历史记录仍保留。"))}>另存历史 .md</button></div></div>
      <div className="ocr-history-result" role="region" aria-label="历史识别正文" tabIndex={0}>{markdown ? <ChatMarkdown content={selected.markdown} /> : <pre>{selected.markdown}</pre>}</div>
      <PerformanceSummary value={selected.performance ? { state: "ready", record: selected.performance.record } : { state: "unavailable" }} image />
    </section>}
  </div></details>;
}
