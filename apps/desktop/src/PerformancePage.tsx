import { useEffect, useRef, useState } from "react";
import type { DesktopApi, PerformanceRecord, PerformanceSnapshot, Snapshot } from "./types";
import { filterPerformance, performanceCsv, performanceStatus, rateText, seconds, tokenRate, validatePerformance } from "./performance";

function Details({ row }: { row: PerformanceRecord }) {
  const p = row.performance;
  return <section className="performance-detail" aria-label="推理详情"><h2>推理详情 · #{row.sequence}</h2><dl>
    <dt>请求 ID</dt><dd>{row.request_id}</dd><dt>模型 ID</dt><dd>{row.model_id}</dd>
    <dt>结果</dt><dd>{performanceStatus(row)}{row.error_code ? ` · ${row.error_code}` : ""}</dd>
    <dt>输入 / 输出 token</dt><dd>{row.usage.prompt_tokens} / {row.usage.completion_tokens}</dd>
    <dt>输出上限</dt><dd>{row.max_output_tokens} token</dd>
    <dt>实际加载参数</dt><dd>{p ? `上下文 ${p.load_options.context_size} · 线程 ${p.load_options.threads} · 批次 ${p.load_options.batch_size}` : "不可用"}</dd>
    <dt>准备耗时</dt><dd>{seconds(p?.timings.prepare_us)}</dd>
    <dt>Prefill（含图片编码）</dt><dd>{seconds(p?.timings.prefill_us)} · {rateText(tokenRate(row, "prefill"))}</dd>
    <dt>Decode</dt><dd>{seconds(p?.timings.decode_us)} · {rateText(tokenRate(row, "decode"))}</dd>
    <dt>同步输出回调</dt><dd>{seconds(p?.timings.output_callback_us)}</dd>
    <dt>排队 / 加载 / 执行</dt><dd>{row.timings.queue_ms} / {row.timings.load_ms} / {row.timings.execution_ms} ms</dd>
  </dl></section>;
}

export function PerformancePage({ api, connection }: { api: DesktopApi; connection: Snapshot["connection"] | undefined }) {
  const [snapshot, setSnapshot] = useState<PerformanceSnapshot | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [copied, setCopied] = useState("");
  const [model, setModel] = useState("");
  const [status, setStatus] = useState("");
  const [modality, setModality] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [owner, setOwner] = useState({ api, connection });
  if (owner.api !== api || owner.connection !== connection) {
    setOwner({ api, connection });
    setSnapshot(null);
    setError("");
    setLoading(false);
    setSelected(null);
    setCopied("");
  }
  const refresh = useRef<() => void>(() => {});
  useEffect(() => {
    let alive = true;
    let pending = false;
    const query = async () => {
      if (!alive || pending || connection !== "connected" || !api.performanceGet || document.visibilityState === "hidden") return;
      pending = true;
      setLoading(true);
      try {
        const result = validatePerformance(await api.performanceGet());
        if (alive) { setSnapshot(result); setError(""); }
      } catch (cause) {
        if (alive) {
          setSnapshot(null);
          const code = cause && typeof cause === "object" && "code" in cause ? cause.code : "";
          setError(["unsupported", "performance_unsupported", "unsupported_service", "unsupported_version"].includes(String(code)) ? "当前版本不支持性能记录，请更新并重启服务。" : "性能查询失败，请手动刷新。聊天和 OCR 结果不受影响。");
        }
      } finally { pending = false; if (alive) setLoading(false); }
    };
    refresh.current = () => { void query(); };
    refresh.current();
    const timer = window.setInterval(() => void query(), 3000);
    const visible = () => { if (document.visibilityState === "visible") void query(); };
    document.addEventListener("visibilitychange", visible);
    return () => { alive = false; clearInterval(timer); document.removeEventListener("visibilitychange", visible); refresh.current = () => {}; };
  }, [api, connection]);
  // Disconnected records never masquerade as measurements of the current service.
  const current = connection === "connected" && !error ? snapshot : null;
  const records = filterPerformance(current?.records ?? [], model, status, modality);
  const key = (row: PerformanceRecord) => `${current?.instance_id}:${row.sequence}`;
  const modelIds = [...new Set(current?.records.map((row) => row.model_id))];
  const detail = records.find((row) => key(row) === selected);
  const empty = connection === "stopped" ? "服务已停止。内存记录随服务退出清空；启动后可查看新推理。" : connection !== "connected" ? "尚未连接运行服务，无法读取性能记录。" : !api.performanceGet ? "当前版本不支持性能记录，请更新桌面应用。" : error || (loading && !current ? "正在读取性能记录…" : current?.records.length ? "没有符合筛选条件的记录。" : "当前服务暂无已结束的推理记录。");
  return <section className="performance-page">
    <div className="page-heading"><div><span className="eyebrow">运行服务 · 实测记录</span><h1>性能</h1><p>对比聊天、图片 OCR、本机校验、本机 API 与局域网 API 的推理。</p></div><div className="workspace-actions"><button disabled={connection !== "connected" || !api.performanceGet || loading} onClick={() => refresh.current()}>{loading ? "正在刷新…" : "刷新性能"}</button><button disabled={!current || records.length === 0} onClick={() => { if (current) void navigator.clipboard.writeText(performanceCsv(current, records)).then(() => setCopied("已复制筛选记录 CSV。")).catch(() => setCopied("复制失败，请重试。")); }}>复制 CSV</button></div></div>
    <p className="small-note">自动记录最近 200 次已结束的推理，停止或重启服务后清空。页面可见时每 3 秒刷新。不保存输入、输出正文、图片或文件路径。</p>
    <div className="performance-filters"><label>模型<select aria-label="性能模型" value={model} onChange={(e) => setModel(e.target.value)}><option value="">全部模型</option>{model && !modelIds.includes(model) && <option value={model}>{model}（当前无记录）</option>}{modelIds.map((id) => <option key={id}>{id}</option>)}</select></label><label>状态<select aria-label="性能状态" value={status} onChange={(e) => setStatus(e.target.value)}><option value="">全部状态</option><option value="completed">成功</option><option value="cancelled">已取消</option><option value="failed">失败</option></select></label><label>输入类型<select aria-label="性能输入类型" value={modality} onChange={(e) => setModality(e.target.value)}><option value="">全部类型</option><option value="text">文本</option><option value="image">图片</option></select></label></div>
    <p role="status">{copied || (current ? `当前服务 ${current.instance_id} · 显示 ${records.length} / ${current.records.length} 条` : "")}</p>
    {records.length ? <div className="performance-table-scroll" role="region" aria-label="近期推理记录" tabIndex={0}><table className="performance-table"><thead><tr><th>记录 / 时间</th><th>模型 / 类型</th><th>结果</th><th>输入 / 输出 token</th><th>Prefill · 含图片编码</th><th>Decode</th></tr></thead><tbody>{records.map((row) => <tr key={key(row)}><td><button aria-pressed={key(row) === selected} onClick={() => setSelected(key(row))}>查看 #{row.sequence}</button><small>{new Date(row.accepted_at_unix_ms).toLocaleString()}</small></td><td>{row.model_id}<small>{row.modality === "image" ? "图片" : "文本"}</small></td><td>{performanceStatus(row)}</td><td>{row.usage.prompt_tokens} / {row.usage.completion_tokens}</td><td>{rateText(tokenRate(row, "prefill"))}<small>{seconds(row.performance?.timings.prefill_us)}</small></td><td>{rateText(tokenRate(row, "decode"))}<small>{seconds(row.performance?.timings.decode_us)}</small></td></tr>)}</tbody></table></div> : <div className="notice-band"><p role="status">{empty}</p></div>}
    {detail && <Details row={detail} />}
    <p className="small-note">Prefill 速度 = 输入 token ÷ prefill 耗时（含图片编码）；Decode 速度 = 输出 token ÷ 生成阶段耗时，已扣同步输出回调时间，仍包含采样、文本处理及清理，并非 llama-bench 纯内核速度。排队、加载、执行分别记录。失败、取消、缺测或零耗时的阶段速度显示“不可用”。相同模型 ID 不代表精确模型 hash 相同，比较时还需核对模型文件、设备及实际参数。</p>
  </section>;
}
