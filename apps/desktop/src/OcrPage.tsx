import { useEffect, useRef, useState } from "react";
import type { DesktopController, ViewState } from "./controller";
import type { ModelFileSelection } from "./types";
import { ChatMarkdown } from "./ChatMarkdown";
import { ModelLoadControl } from "./ModelLoadControl";
import { prepareOcrImage } from "./ocrImage";
import { safeError } from "./adapter";
import { validModelSelection } from "./modelSelection";

export function OcrPage({ controller, state }: { controller: DesktopController; state: ViewState }) {
  const api = controller.api;
  const [model, setModel] = useState("");
  const [imageSelection, setImageSelection] = useState<{ file: File; edge: number; generation: number } | null>(null);
  const file = imageSelection?.file ?? null;
  const [image, setImage] = useState("");
  const [imageError, setImageError] = useState("");
  const imageInput = useRef<HTMLInputElement>(null);
  const imageGeneration = useRef(0);
  const preparedGeneration = useRef(-1);
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
  const [pairStatus, setPairStatus] = useState("");
  const [selecting, setSelecting] = useState(false);
  const [importing, setImporting] = useState(false);
  const pairOperation = useRef(false);
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
  const selectImage = (nextFile: File | null, nextEdge: number) => {
    // Invalidate the previous preparation before its promise can settle.
    const generation = ++imageGeneration.current;
    setImage("");
    setImageError("");
    setStatus("");
    setPreparing(!!nextFile);
    setEdge(nextEdge);
    setImageSelection(nextFile ? { file: nextFile, edge: nextEdge, generation } : null);
  };
  const pickPair = async () => {
    if (!api.pickModelPair || pairOperation.current || busy || blocked) return;
    pairOperation.current = true;
    setPair(null);
    setSelecting(true);
    setPairStatus("正在选择配套文件：先选择主模型，再在第二个窗口选择 mmproj 视觉投影。");
    try {
      const selection = await api.pickModelPair();
      if (!selection) {
        setPairStatus("已取消文件选择，尚未导入。请重新选择两个配套文件。");
      } else if (!validModelSelection(selection) || selection.files.length !== 2) {
        setPairStatus("文件选择结果不完整，请重新选择主模型和视觉投影。（response_invalid）");
      } else {
        setPair(selection);
        setPairStatus("两个文件已选好。请核对下方文件名，填写模型 ID，再点击“复制并导入两个文件”。");
      }
    } catch (error) {
      const failure = safeError(error);
      setPairStatus(`选择失败：${failure.message}（${failure.code}）`);
    } finally {
      pairOperation.current = false;
      setSelecting(false);
    }
  };
  const importPair = async () => {
    if (!api.importModelPair || !pair || !pairId.trim() || pairOperation.current || busy || blocked || state.snapshot?.connection !== "connected") return;
    pairOperation.current = true;
    setImporting(true);
    setPairStatus("正在复制并校验两个模型文件，请等待。完成后还需要加载模型。");
    try {
      await api.importModelPair(pair.selection_id, pairId.trim());
      setPair(null);
      setPairStatus("双文件导入完成，源文件保留。请在上方“OCR 模型”中选择它，再点击“加载所选 OCR 模型”。");
      await controller.refresh();
      await controller.refreshModels();
    } catch (error) {
      const failure = safeError(error);
      setPair(null);
      setPairStatus(`导入未完成：${failure.message}（${failure.code}）请重新选择两个文件后再试。`);
    } finally {
      pairOperation.current = false;
      setImporting(false);
    }
  };
  useEffect(() => () => { const current = task.current; if (current) { current.cancelled = true; if (current.id) void api.chatCancel(current.id).catch(() => {}); } }, [api]);
  useEffect(() => {
    let active = true;
    if (imageSelection) {
      const current = () => active && imageGeneration.current === imageSelection.generation;
      void prepareOcrImage(imageSelection.file, imageSelection.edge).then((value) => {
        if (!current()) return;
        preparedGeneration.current = imageSelection.generation;
        setImage(value);
        setImageError("");
      }).catch((error: unknown) => {
        if (current()) setImageError(error instanceof Error ? error.message : "图片准备失败，请重新选择图片。");
      }).finally(() => { if (current()) setPreparing(false); });
    }
    return () => { active = false; };
  }, [imageSelection]);
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
    if (!api.ocrStart || !ready || !image || preparing || preparedGeneration.current !== imageGeneration.current || task.current || busy || pairOperation.current || blocked || !prompt.trim() || !Number.isInteger(tokens) || tokens < 1 || tokens > 4096 || !outputFits) return;
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
    <div className="page-heading"><div><span className="eyebrow">本机单页识别</span><h1>图片 OCR</h1><p>选择图片，提取文字并保存为 Markdown。</p></div><span className="subtle-pill">{ready ? "OCR 模型已就绪" : "等待加载 OCR 模型"}</span></div>
    <section className="ocr-card ocr-model-card" aria-labelledby="ocr-model-heading">
      <div className="ocr-card-heading"><span className="ocr-step" aria-hidden="true">1</span><div><h2 id="ocr-model-heading">准备模型</h2><p>首次使用先导入两个配套文件，之后选择已导入的模型加载。</p></div></div>
      <fieldset aria-label="OCR 模型准备" disabled={busy || selecting || importing || blocked}>
        <div className="ocr-model-row">
          <label className="ocr-model-select">OCR 模型<select aria-label="OCR 模型" value={model} onChange={(event) => setModel(event.target.value)}><option value="">{models.length ? "选择模型" : "暂无 OCR 模型，请先在下方导入"}</option>{models.map((item) => <option key={item.id} value={item.id}>{item.display_name} · 视觉配对</option>)}</select></label>
          <button disabled={!selected || !selected.available || !selected.loadable || !loadValid} onClick={() => void controller.loadModel(model, { context_size: contextSize, batch_size: batchSize, threads })}>加载所选 OCR 模型</button>
        </div>
        <div className="ocr-parameter-grid">
          <label>本次上下文<input aria-label="OCR 加载上下文" type="number" min={512} value={contextSize} onChange={(event) => setContextSize(Number(event.target.value))} /></label>
          <label>本次批次<input aria-label="OCR 加载批次" type="number" min={1} value={batchSize} onChange={(event) => setBatchSize(Number(event.target.value))} /></label>
          <label>本次线程<input aria-label="OCR 加载线程" type="number" min={1} max={64} value={threads} onChange={(event) => setThreads(Number(event.target.value))} /></label>
        </div>
        <p className="small-note">建议 8192 / 256 / 4；i5-8400 可对比 6 线程。修改后点击加载才生效，不改写已保存档案。</p>
        <details className="ocr-import" open={models.length === 0 || pairStatus ? true : undefined}>
          <summary>导入 OCR 模型与视觉投影</summary>
          <div className="ocr-import-content">
            <p className="small-note">先选主模型 GGUF，再选配套 mmproj GGUF；两个文件不必放在同一文件夹。导入需要先启动运行服务，并会复制文件，保留原件。</p>
            <button disabled={!api.pickModelPair} onClick={() => void pickPair()}>{selecting ? "正在选择配套文件…" : "选择两个配套文件"}</button>
            <p className="ocr-feedback" role="status" aria-label="OCR 模型导入状态">{pairStatus || "尚未选择文件。选好后会在这里显示两个文件名。"}</p>
            {pair && <><ul className="ocr-pair-files">{pair.files.map((item) => <li key={item.selection_index}>{item.selection_index === 0 ? "主模型" : "视觉投影"}：{item.file_name}</li>)}</ul><div className="ocr-import-row"><label>模型 ID<input value={pairId} placeholder="例如 glm-ocr-q8" onChange={(event) => setPairId(event.target.value)} /></label><button disabled={!pairId.trim() || !api.importModelPair || state.snapshot?.connection !== "connected"} onClick={() => void importPair()}>{importing ? "正在复制并校验…" : "复制并导入两个文件"}</button></div></>}
          </div>
        </details>
      </fieldset>
      <ModelLoadControl task={state.model_load} controller={controller} />
    </section>
    <div className="ocr-workspace">
      <section className="ocr-card ocr-image-card" aria-labelledby="ocr-image-heading">
        <div className="ocr-card-heading"><span className="ocr-step" aria-hidden="true">2</span><div><h2 id="ocr-image-heading">选择图片</h2><p>一次识别一张图片，建议先用清晰的文字截图测试。</p></div></div>
        <fieldset aria-label="OCR 图片与识别设置" disabled={busy || selecting || importing || blocked}>
          <div className="ocr-upload">
            <div><label htmlFor="ocr-image-input">图片文件</label><p id="ocr-image-format" className="small-note">PNG / JPEG，最多 4 MiB</p></div>
            <button type="button" aria-describedby="ocr-image-format ocr-image-feedback" onClick={() => imageInput.current?.click()}>选择图片</button>
            <input ref={imageInput} id="ocr-image-input" hidden aria-label="OCR 图片" type="file" accept=".png,.jpg,.jpeg,image/png,image/jpeg" aria-describedby="ocr-image-feedback" onChange={(event) => { const nextFile = event.target.files?.[0] ?? null; selectImage(nextFile, edge); event.target.value = ""; }} />
          </div>
          <div className="ocr-image-preview">
            <div className="ocr-preview-frame">{image ? <img className="ocr-preview" src={image} alt="待识别图片预览" /> : <div className="ocr-empty-state"><strong>{preparing ? "正在准备图片…" : imageError ? "图片未能准备完成" : "图片预览"}</strong><p>{preparing ? "正在读取和检查图片，请稍候。" : imageError ? "请查看下方原因，重新选择图片后再试。" : "选择图片后，在这里检查方向和文字清晰度。"}</p></div>}</div>
            <div id="ocr-image-feedback" className={`ocr-feedback ocr-image-feedback${imageError ? " warning-text" : ""}`} role="status" aria-label="OCR 图片准备状态">
              {file && <p className="ocr-selected-image">已选择：{file.name}</p>}
              <p>{imageError || (preparing ? edge ? "正在检查并准备所选尺寸的图片…" : "正在检查原图…" : image ? edge ? "图片已准备完成，预览为本次将发送的图片。" : "原图已准备完成，可在预览中核对文字清晰度。" : "尚未选择图片。")}</p>
            </div>
          </div>
          <div className="ocr-image-settings">
            <label>发送图片尺寸<select aria-label="发送图片尺寸" value={edge} onChange={(event) => selectImage(file, Number(event.target.value))}><option value={0}>原图（不缩放）</option><option value={1600}>最长边 1600</option><option value={2048}>最长边 2048</option></select></label>
            <label>最大输出 token<input aria-label="最大输出 token" type="number" min={1} max={4096} value={tokens} onChange={(event) => setTokens(Number(event.target.value))} /></label>
          </div>
          <p className="small-note">缩小图片可能丢失小字；输出达到上限时会提示内容可能截断。</p>
          <label className="ocr-prompt">识别提示词<textarea value={prompt} rows={2} maxLength={4096} onChange={(event) => setPrompt(event.target.value)} /></label>
        </fieldset>
        <div className="ocr-run-actions"><button className="primary" disabled={!ready || !image || busy || selecting || importing || preparing || blocked || !prompt.trim() || tokens < 1 || tokens > 4096 || !Number.isInteger(tokens) || !outputFits} onClick={() => void start()}>识别图片</button><button disabled={!busy} onClick={() => void stop()}>停止识别</button>{recovering && <button onClick={() => void recover()}>重新确认识别任务</button>}</div>
        <p className="ocr-feedback" role="status">{status || (!ready ? "请先加载所选 OCR 模型。" : preparing ? "图片正在准备，请稍候。" : imageError ? "图片准备失败，请重新选择图片。" : image ? "模型和图片已就绪，可开始识别。" : "模型已加载，请先选择图片。")}</p>
        <p className="small-note">当前加载上下文：{runtime?.selected_model === model ? runtime?.load_options?.context_size ?? "未知" : "未加载"}。图片、提示词和输出共同占用上下文。</p>
      </section>
      <section className="ocr-card ocr-output-card" aria-labelledby="ocr-output-heading">
        <div className="ocr-card-heading"><span className="ocr-step" aria-hidden="true">3</span><div><h2 id="ocr-output-heading">识别结果</h2><p>文字会逐步显示，完成后请对照原图核对。</p></div></div>
        <div className="ocr-result-toolbar">
          <div className="ocr-view-switch" role="group" aria-label="结果显示方式"><button aria-pressed={!markdown} onClick={() => setMarkdown(false)}>原文</button><button aria-pressed={markdown} onClick={() => setMarkdown(true)}>Markdown</button></div>
          <div className="ocr-export-actions"><button disabled={!text} onClick={() => void navigator.clipboard.writeText(text).then(() => setStatus("已复制原文。")).catch(() => setStatus("复制失败，请手动复制原文。"))}>复制识别原文</button><button disabled={!text || !api.saveOcrMarkdown} onClick={() => void api.saveOcrMarkdown?.(text).then((result) => setStatus(result.saved ? "已保存 Markdown 原文。" : "已取消保存。")).catch(() => setStatus("保存失败，原文仍保留。"))}>保存 .md</button></div>
        </div>
        <div className="ocr-result-body" tabIndex={0} role="region" aria-label="识别结果内容">{text ? markdown ? <ChatMarkdown content={text} /> : <pre className="ocr-result">{text}</pre> : <div className="ocr-empty-state"><strong>识别结果会显示在这里</strong><p>加载模型并选择图片后，点击“识别图片”。</p></div>}</div>
      </section>
    </div>
  </section>;
}
