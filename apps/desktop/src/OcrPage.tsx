import { useEffect, useRef, useState } from "react";
import type { DesktopController, ViewState } from "./controller";
import type { ModelFileSelection } from "./types";
import { ChatMarkdown } from "./ChatMarkdown";
import { ModelLoadControl } from "./ModelLoadControl";
import { prepareOcrImage } from "./ocrImage";

export function OcrPage({ controller, state }: { controller: DesktopController; state: ViewState }) {
  const api = controller.api;
  const [model, setModel] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [image, setImage] = useState("");
  const [edge, setEdge] = useState(0);
  const [prompt, setPrompt] = useState("Text Recognition:");
  const [tokens, setTokens] = useState(2048);
  const [contextSize, setContextSize] = useState(8192);
  const [batchSize, setBatchSize] = useState(256);
  const [threads, setThreads] = useState(4);
  const [text, setText] = useState("");
  const [markdown, setMarkdown] = useState(false);
  const [busy, setBusy] = useState(false);
  const [recovering, setRecovering] = useState(false);
  const [status, setStatus] = useState("");
  const [pair, setPair] = useState<ModelFileSelection | null>(null);
  const [pairId, setPairId] = useState("");
  const [importing, setImporting] = useState(false);
  const task = useRef<{ id: string | null; cancelled: boolean; reading: boolean; hadReadFailure: boolean } | null>(null);
  const [preparing, setPreparing] = useState(false);
  const models = state.models?.data.filter((item) => item.has_projector) ?? [];
  const selected = models.find((item) => item.id === model);
  const runtime = state.snapshot?.runtime;
  const loadValid = Number.isInteger(contextSize) && contextSize >= 512 && contextSize <= (selected?.context_limit ?? 131072) && Number.isInteger(batchSize) && batchSize >= 1 && batchSize <= contextSize && Number.isInteger(threads) && threads >= 1 && threads <= 64;
  const activeContext = runtime?.selected_model === model ? runtime?.load_options?.context_size : null;
  const outputFits = activeContext == null || tokens < activeContext;
  const ready = runtime?.state === "ready" && runtime.selected_model === model;
  const blocked = !!state.operation || state.chat_phase !== "idle" || state.library_phase !== "idle" || state.download_phase !== "idle";
  useEffect(() => () => { const current = task.current; if (current) { current.cancelled = true; if (current.id) void api.chatCancel(current.id).catch(() => {}); } }, [api]);
  useEffect(() => {
    let active = true;
    if (file) void prepareOcrImage(file, edge).then((value) => { if (active) setImage(value); }).catch((error: Error) => { if (active) setStatus(error.message); }).finally(() => { if (active) setPreparing(false); });
    return () => { active = false; };
  }, [file, edge]);
  const stop = async () => {
    const current = task.current;
    if (!current) return;
    current.cancelled = true;
    setStatus("停止中…");
    try { if (current.id) await api.chatCancel(current.id); } catch { setStatus("停止未确认，请重试停止。"); }
  };
  const consume = async (current: NonNullable<typeof task.current>) => {
    if (!current.id || current.reading) return;
    current.reading = true;
    setRecovering(false);
    try {
      let terminal = false;
      while (!terminal) {
        const batch = await api.chatNext(current.id);
        if (batch.request_id !== current.id) throw new Error("响应标识不一致");
        const terminalIndex = batch.events.findIndex((event) => ["completed", "cancelled", "failed"].includes(event.type));
        if (batch.terminal !== (terminalIndex >= 0) || (terminalIndex >= 0 && terminalIndex !== batch.events.length - 1)) throw new Error("终态回执不完整或终态后仍有事件");
        for (const event of batch.events) {
          if (event.type === "delta") setText((value) => value + event.text);
          if (event.type === "completed") setStatus(event.finish_reason === "length" ? `已达到输出 token 上限，内容可能截断；请核对原图，可调整上限后手动重试。${current.hadReadFailure ? "读取曾中断，原文还可能缺失内容。" : ""}` : current.hadReadFailure ? "终态已确认，但读取曾中断，原文可能缺失内容；请核对原图。" : "识别完成，请核对原图。");
          if (event.type === "cancelled") setStatus("已停止，已生成内容可能不完整。");
          if (event.type === "failed") setStatus(`识别失败：${event.message}（${event.code}）。已生成内容可能不完整。`);
        }
        terminal = batch.terminal;
      }
      // Only consuming this owned request's terminal releases the shared slot.
      task.current = null;
      setBusy(false);
      void controller.refresh();
    } catch {
      current.hadReadFailure = true;
      current.cancelled = true;
      await api.chatCancel(current.id).catch(() => {});
      setStatus("识别结果与停止尚未确认，已保留任务。请重新确认识别任务；不会自动重新生成。");
      setRecovering(true);
    } finally { current.reading = false; }
  };
  const recover = async () => {
    const current = task.current;
    if (!current?.id || current.reading) return;
    setRecovering(false);
    setStatus("正在确认原识别任务…");
    // A failed cancel ACK must not prevent reading the authoritative terminal.
    await api.chatCancel(current.id).catch(() => {});
    await consume(current);
  };
  const start = async () => {
    if (!api.ocrStart || !ready || !image || busy || blocked || !prompt.trim() || !Number.isInteger(tokens) || tokens < 1 || tokens > 4096 || !outputFits) return;
    const current = { id: null as string | null, cancelled: false, reading: false, hadReadFailure: false };
    task.current = current;
    setBusy(true); setRecovering(false); setText(""); setStatus("正在识别…");
    try {
      current.id = (await api.ocrStart({ model_id: model, image_data_url: image, prompt, max_output_tokens: tokens })).request_id;
      if (!current.id) throw new Error("识别请求未返回有效标识。");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "识别未能开始，未自动重试。");
      task.current = null; setBusy(false); void controller.refresh();
      return;
    }
    if (current.cancelled) await api.chatCancel(current.id).catch(() => {});
    await consume(current);
  };
  return <section className="ocr-page">
    <div className="page-heading"><div><span className="eyebrow">本机单页识别</span><h1>图片 OCR</h1></div></div>
    <p>选择已配对视觉投影的模型，显式加载后识别一张图片。加载成功不代表 OCR 已验证；识别结果需要人工核对。</p>
    <fieldset disabled={busy || importing || blocked}>
      <label>OCR 模型<select aria-label="OCR 模型" value={model} onChange={(event) => setModel(event.target.value)}><option value="">选择模型</option>{models.map((item) => <option key={item.id} value={item.id}>{item.display_name} · 视觉配对</option>)}</select></label>
      <button disabled={!selected || !selected.available || !selected.loadable || !loadValid} onClick={() => void controller.loadModel(model, { context_size: contextSize, batch_size: batchSize, threads })}>加载所选 OCR 模型</button>
      <p className="small-note">本次加载建议：上下文 8192、批次 256、线程 4（可按 CPU 改为 6）。下列值仅用于这次显式加载，不覆盖已保存档案。图片、提示词和输出共同占用上下文；模型上限：{selected?.context_limit ?? "未知"}。</p>
      <label>本次上下文<input aria-label="OCR 加载上下文" type="number" min={512} value={contextSize} onChange={(event) => setContextSize(Number(event.target.value))} /></label>
      <label>本次批次<input aria-label="OCR 加载批次" type="number" min={1} value={batchSize} onChange={(event) => setBatchSize(Number(event.target.value))} /></label>
      <label>本次线程<input aria-label="OCR 加载线程" type="number" min={1} max={64} value={threads} onChange={(event) => setThreads(Number(event.target.value))} /></label>
      <details><summary>导入 OCR 模型与视觉投影</summary><p>分别选择主 GGUF 和配套视觉 GGUF（mmproj）。将复制两个文件到托管模型目录，源文件保留；单文件添加仍为零复制。导入需要运行服务已启动。</p>
        <button disabled={!api.pickModelPair} onClick={() => { setPair(null); void api.pickModelPair?.().then(setPair).catch(() => setStatus("选择模型失败。")); }}>选择两个配套文件</button>
        {pair && <><ul>{pair.files.map((item) => <li key={item.selection_index}>{item.selection_index === 0 ? "主模型" : "视觉投影"}：{item.file_name}</li>)}</ul><label>模型 ID<input value={pairId} onChange={(event) => setPairId(event.target.value)} /></label><button disabled={!pairId.trim() || !api.importModelPair || state.snapshot?.connection !== "connected"} onClick={() => { setImporting(true); void api.importModelPair!(pair.selection_id, pairId.trim()).then(() => { setPair(null); setStatus("双文件导入完成，源文件保留。"); return controller.refresh(); }).catch(() => { setPair(null); setStatus("导入未完成，请检查运行服务并重新选择文件。"); }).finally(() => setImporting(false)); }}>复制并导入两个文件</button></>}
      </details>
      <label>图片（PNG / JPEG，最多 4 MiB）<input aria-label="OCR 图片" type="file" accept="image/png,image/jpeg" onChange={(event) => { setStatus(""); setImage(""); setPreparing(!!event.target.files?.[0]); setFile(event.target.files?.[0] ?? null); }} /></label>
      <label>发送图片尺寸<select aria-label="发送图片尺寸" value={edge} onChange={(event) => { setImage(""); setPreparing(!!file); setEdge(Number(event.target.value)); }}><option value={0}>原图（不缩放）</option><option value={1600}>最长边 1600（缩放可能丢失细节）</option><option value={2048}>最长边 2048（缩放可能丢失细节）</option></select></label>
      <label>识别提示词<textarea value={prompt} maxLength={4096} onChange={(event) => setPrompt(event.target.value)} /></label>
      <label>最大输出 token<input aria-label="最大输出 token" type="number" min={1} max={4096} value={tokens} onChange={(event) => setTokens(Number(event.target.value))} /></label>
    </fieldset>
    <ModelLoadControl task={state.model_load} controller={controller} />
    {image && <img className="ocr-preview" src={image} alt="待识别图片预览" />}
    <div className="workspace-actions"><button className="primary" disabled={!ready || !image || busy || preparing || blocked || !prompt.trim() || tokens < 1 || tokens > 4096 || !Number.isInteger(tokens) || !outputFits} onClick={() => void start()}>识别图片</button><button disabled={!busy} onClick={() => void stop()}>停止识别</button>{recovering && <button onClick={() => void recover()}>重新确认识别任务</button>}</div>
    <p className="small-note">当前模型加载上下文：{runtime?.selected_model === model ? runtime?.load_options?.context_size ?? "未知" : "未加载"}。输出还需为图像与提示词留出空间；总 token 超限时会明确报错，不自动截断或重新加载。</p>
    <p role="status">{status || (ready ? "模型已加载，可开始识别。" : "请先加载所选 OCR 模型。")}</p>
    <div className="workspace-actions"><button aria-pressed={!markdown} onClick={() => setMarkdown(false)}>原文</button><button aria-pressed={markdown} onClick={() => setMarkdown(true)}>Markdown</button><button disabled={!text} onClick={() => void navigator.clipboard.writeText(text).then(() => setStatus("已复制原文。")).catch(() => setStatus("复制失败，请手动复制原文。"))}>复制识别原文</button><button disabled={!text || !api.saveOcrMarkdown} onClick={() => void api.saveOcrMarkdown?.(text).then((result) => setStatus(result.saved ? "已保存 Markdown 原文。" : "已取消保存。")).catch(() => setStatus("保存失败，原文仍保留。"))}>保存 .md</button></div>
    {markdown ? <ChatMarkdown content={text} /> : <pre className="ocr-result">{text}</pre>}
  </section>;
}
