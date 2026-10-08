import { PerformanceSummary } from "./PerformanceSummary";
import { OcrHistory } from "./OcrHistory";
import { readRequestPerformance } from "./performance";
import type { RequestPerformance } from "./performance";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { DesktopController, ViewState } from "./controller";
import type { LoadOptions, ModelFileSelection, OcrHistorySaveRequest, Usage } from "./types";
import { ChatMarkdown } from "./ChatMarkdown";
import { ModelLoadControl } from "./ModelLoadControl";
import { prepareOcrImage } from "./ocrImage";
import { safeError } from "./adapter";
import { validModelSelection } from "./modelSelection";
import { OCR_DEFAULT_LOAD, resolveOcrProfile, validLoad } from "./workbench";
import { ownRecord } from "./records";

type QueueItem = { id: number; file: File; status: "待开始" | "准备图片" | "识别中" | "已完成" | "已停止" | "失败"; text: string; notice: string; performance: RequestPerformance | null; usage?: Usage; incomplete?: boolean };
type BatchRun = { token: symbol; stopped: boolean; model: string; prompt: string; tokens: number; edge: number; done: Promise<void> };

export function OcrPage({ controller, state }: { controller: DesktopController; state: ViewState }) {
  const api = controller.api;
  const workbench = useSyncExternalStore(controller.subscribe, controller.getSnapshot).workbench;
  const prefs = workbench.preferences.ocr;
  const model = prefs.model_id ?? "";
  const { prompt, max_output_tokens: tokens, context_size: contextSize, batch_size: batchSize, threads, image_edge: edge, markdown } = prefs;
  const [profile, setProfile] = useState<{ id: string; base: LoadOptions; conflict: boolean; error: boolean } | null>(null);
  const profilePending = !!model && !!api.configurationModelGet && profile?.id !== model;
  const profileConflict = profile?.id === model && profile.conflict;
  const profileError = profile?.id === model && profile.error;
  const setModel = (id: string) => {
    const draft = ownRecord(prefs.model_drafts, id)?.draft ?? OCR_DEFAULT_LOAD;
    controller.setOcrPreferences({ model_id: id || null, ...draft });
  };
  const setLoadOptions = (patch: Partial<LoadOptions>) => {
    const draft = { context_size: contextSize, batch_size: batchSize, threads, ...patch };
    const base = profileConflict ? ownRecord(prefs.model_drafts, model)?.base ?? OCR_DEFAULT_LOAD : profile?.id === model ? profile.base : ownRecord(prefs.model_drafts, model)?.base ?? OCR_DEFAULT_LOAD;
    controller.setOcrPreferences({ ...draft, ...(model ? { model_drafts: { ...prefs.model_drafts, [model]: { base, draft } } } : {}) });
  };
  const resolveConflict = (keep: boolean) => {
    if (!profile || profile.id !== model) return;
    const draft = keep ? { context_size: contextSize, batch_size: batchSize, threads } : profile.base;
    controller.setOcrPreferences({ ...draft, model_drafts: { ...prefs.model_drafts, [model]: { base: profile.base, draft } } });
    setProfile({ ...profile, conflict: false, error: false });
  };
  useEffect(() => {
    let alive = true;
    if (model && api.configurationModelGet && workbench.hydrated) {
      void api.configurationModelGet(model).then((config) => {
        if (!alive) return;
        if (config.model_id !== model || !validLoad(config.saved_effective)) throw new Error("invalid_model_configuration");
        const latest = controller.getSnapshot().workbench.preferences.ocr;
        const saved = ownRecord(latest.model_drafts, model);
        const resolved = resolveOcrProfile(config, saved);
        controller.setOcrPreferences({ ...resolved.draft, ...(saved && !resolved.conflict ? { model_drafts: { ...latest.model_drafts, [model]: { base: resolved.base, draft: resolved.draft } } } : {}) });
        setProfile({ id: model, base: resolved.base, conflict: resolved.conflict, error: false });
      }).catch(() => { if (alive) setProfile({ id: model, base: OCR_DEFAULT_LOAD, conflict: false, error: true }); });
    }
    return () => { alive = false; };
  }, [api, controller, model, workbench.hydrated, state.snapshot?.configuration?.revision]);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const queueRef = useRef<QueueItem[]>([]);
  const nextItem = useRef(0);
  const batchRun = useRef<BatchRun | null>(null);
  const [settingsFrozen, setSettingsFrozen] = useState(false);
  const frozen = useRef<{ model: string; prompt: string; tokens: number; edge: number } | null>(null);
  const [previewItem, setPreviewItem] = useState<number | null>(null);
  const [viewed, setViewed] = useState<number | null>(null);
  const viewedRef = useRef<number | null>(null);
  const updateItem = useCallback((id: number, patch: Partial<QueueItem>) => {
    queueRef.current = queueRef.current.map((item) => item.id === id ? { ...item, ...patch } : item);
    setQueue(queueRef.current);
  }, []);
  const [imageSelection, setImageSelection] = useState<{ file: File; edge: number; generation: number } | null>(null);
  const file = imageSelection?.file ?? null;
  const [image, setImage] = useState("");
  const [imageError, setImageError] = useState("");
  const imageInput = useRef<HTMLInputElement>(null);
  const imageGeneration = useRef(0);
  const preparedGeneration = useRef(-1);
  const [text, setText] = useState("");
  const [performance, setPerformance] = useState<RequestPerformance | null>(null);
  const [usage, setUsage] = useState<Usage | undefined>();
  const performanceEpoch = useRef(0);
  const lifecycle = useRef(0);
  const [historyChanged, setHistoryChanged] = useState(0);
  const [historyNotice, setHistoryNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [recovering, setRecovering] = useState(false);
  const [status, setStatus] = useState("");
  const [pair, setPair] = useState<ModelFileSelection | null>(null);
  const [pairId, setPairId] = useState("");
  const [pairStatus, setPairStatus] = useState("");
  const [selecting, setSelecting] = useState(false);
  const [importing, setImporting] = useState(false);
  const pairOperation = useRef(false);
  const task = useRef<{ item: number; overflow: boolean; id: string | null; cancelled: boolean; reading: boolean; hadReadFailure: boolean; model_id: string; max_output_tokens: number; epoch: number; lifecycle: number; image_name: string; content: string; started: Promise<void>; markStarted: () => void; readWaiters: Set<() => void> } | null>(null);
  const [preparing, setPreparing] = useState(false);
  const models = state.models?.data.filter((item) => item.has_projector) ?? [];
  const selected = models.find((item) => item.id === model);
  const runtime = state.snapshot?.runtime;
  const loadValid = validLoad({ context_size: contextSize, batch_size: batchSize, threads }) && contextSize <= (selected?.context_limit ?? 131072);
  const activeContext = runtime?.selected_model === model ? runtime?.load_options?.context_size : null;
  const outputFits = activeContext == null || tokens < activeContext;
  const ready = runtime?.state === "ready" && runtime.selected_model === model;
  const blocked = state.closing || !!state.operation || state.chat_phase !== "idle" || state.library_phase !== "idle" || state.download_phase !== "idle";
  const selectImage = (nextFile: File | null, nextEdge: number) => {
    // Invalidate the previous preparation before its promise can settle.
    const generation = ++imageGeneration.current;
    setImage("");
    setImageError("");
    setStatus("");
    setPreparing(!!nextFile);
    controller.setOcrPreferences({ image_edge: nextEdge as 0 | 1600 | 2048 });
    setImageSelection(nextFile ? { file: nextFile, edge: nextEdge, generation } : null);
  };
  const chooseFiles = (files: File[]) => {
    if (batchRun.current && controller.ownsOcrBatch(batchRun.current.token) || controller.getSnapshot().closing) return;
    if (files.length > 20 || files.reduce((total, item) => total + item.size, 0) > 80 * 1024 * 1024 || files.some((item) => item.size > 4 * 1024 * 1024)) {
      setImageError("最多选择 20 张图片，单张不能超过 4 MiB，合计不能超过 80 MiB；原队列已保留。"); return;
    }
    ++performanceEpoch.current;
    frozen.current = null; setSettingsFrozen(false); setPreviewItem(null);
    batchRun.current = null;
    const items: QueueItem[] = files.map((file) => ({ id: ++nextItem.current, file, status: "待开始", text: "", notice: "", performance: null }));
    queueRef.current = items; setQueue(items); setViewed(null); viewedRef.current = null;
    setText(""); setPerformance(null); setUsage(undefined); setHistoryNotice("");
    selectImage(items.length === 1 ? items[0].file : null, edge);
  };
  const editQueue = (id: number, direction: -1 | 0 | 1) => {
    if (frozen.current || batchRun.current && controller.ownsOcrBatch(batchRun.current.token)) return;
    const items = [...queueRef.current]; const index = items.findIndex((item) => item.id === id);
    if (direction === 0) items.splice(index, 1);
    else if (index + direction >= 0 && index + direction < items.length) [items[index], items[index + direction]] = [items[index + direction], items[index]];
    queueRef.current = items; setQueue(items);
    selectImage(items.length === 1 ? items[0].file : null, edge);
  };
  const viewResult = (item: QueueItem) => {
    viewedRef.current = item.id; setViewed(item.id); setText(item.text); setPerformance(item.performance); setUsage(item.usage); setHistoryNotice(item.notice);
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
  useEffect(() => () => { ++performanceEpoch.current; ++lifecycle.current; if (batchRun.current) { batchRun.current.stopped = true; controller.releaseOcrBatch(batchRun.current.token); } const current = task.current; if (current) { current.cancelled = true; if (current.id) void api.chatCancel(current.id).catch(() => {}); } }, [api, controller]);
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
    if (batchRun.current) batchRun.current.stopped = true;
    const current = task.current;
    if (!current) { setStatus("已停止批量识别，剩余图片未开始。"); return; }
    current.cancelled = true;
    setStatus("停止中…");
    try { if (current.id) await api.chatCancel(current.id); } catch { setStatus("停止未确认，请重试停止。"); }
  };
  const consume = useCallback(async (current: NonNullable<typeof task.current>) => {
    if (!current.id || current.reading) return;
    current.reading = true;
    setRecovering(false);
    let advance = false;
    try {
      let terminal = false;
      while (!terminal) {
        const batch = await api.chatNext(current.id);
        if (current.epoch !== performanceEpoch.current) return;
        if (batch.request_id !== current.id) throw new Error("响应标识不一致");
        const terminalIndex = batch.events.findIndex((event) => ["completed", "cancelled", "failed"].includes(event.type));
        if (batch.terminal !== (terminalIndex >= 0) || (terminalIndex >= 0 && terminalIndex !== batch.events.length - 1)) throw new Error("终态回执不完整或终态后仍有事件");
        for (const event of batch.events) {
          if (event.type === "delta" && !current.overflow) {
            const content = current.content + event.text;
            if (new TextEncoder().encode(content).length > 256 * 1024) {
              current.overflow = true; current.cancelled = true; current.hadReadFailure = true;
              if (batchRun.current) batchRun.current.stopped = true;
              void api.chatCancel(current.id).catch(() => {});
              setStatus("本张正文达到 256 KiB 上限，已停止并保留上限内的部分内容；批量已暂停。");
            } else { current.content = content; updateItem(current.item, { text: content }); if (viewedRef.current === current.item) setText(content); }
          }
          if (event.type === "completed") setStatus(event.finish_reason === "length" ? `已达到输出 token 上限，内容可能截断；请核对原图，可调整上限后手动重试。${current.hadReadFailure ? "读取曾中断，原文还可能缺失内容。" : ""}` : current.hadReadFailure ? "终态已确认，但读取曾中断，原文可能缺失内容；请核对原图。" : "识别完成，请核对原图。");
          if (event.type === "cancelled") setStatus("已停止，已生成内容可能不完整。");
          if (event.type === "failed") setStatus(event.code === "execution_timeout" ? "推理执行超时（execution_timeout）。已保留已生成内容，但可能不完整，不会自动重试。可将图片裁成较小区域分别识别，或停止服务后在“设置→资源与校验”中适当调大“推理执行超时”，再启动服务、重新加载模型并手动重试。" : `识别失败：${event.message}（${event.code}）。已生成内容可能不完整。`);
        }
        terminal = batch.terminal;
        const terminalEvent = batch.events[terminalIndex];
        if (terminal && terminalEvent && (terminalEvent.type === "completed" || terminalEvent.type === "failed" || terminalEvent.type === "cancelled")) {
          const savedRequest: OcrHistorySaveRequest = {
            mode: "create", id: current.id, model_id: current.model_id, image_name: current.image_name,
            status: terminalEvent.type, finish_reason: terminalEvent.type === "completed" ? terminalEvent.finish_reason : null,
            error_code: terminalEvent.type === "failed" ? terminalEvent.code : terminalEvent.type === "cancelled" ? "request_cancelled" : null,
            incomplete: current.hadReadFailure || terminalEvent.type !== "completed" || terminalEvent.finish_reason === "length",
            markdown: current.content, performance: null,
          };
          const notice = (text: string) => { updateItem(current.item, { notice: text }); if (viewedRef.current === current.item) setHistoryNotice(text); };
          const changed = () => { if (lifecycle.current === current.lifecycle) setHistoryChanged((value) => value + 1); };
          // Persist terminal text independently: a missing or slow metrics read cannot lose it.
          let saved: Promise<boolean> = Promise.resolve(false);
          if (savedRequest.markdown.trim()) {
            if (api.ocrHistorySave) {
              notice("正在保存到最近识别结果…");
              saved = controller.persistOcrHistory(savedRequest).then((success) => { if (success) { notice("已保存到本机最近识别结果。"); changed(); } else notice("本机历史保存失败，当前正文仍保留，请复制或另存 Markdown。"); return success; });
            } else notice("当前桌面版本不支持自动保存识别历史，当前结果仍可复制或另存。");
          }
          updateItem(current.item, { status: terminalEvent.type === "completed" ? "已完成" : terminalEvent.type === "cancelled" ? "已停止" : "失败", incomplete: savedRequest.incomplete, performance: { state: "pending" }, usage: terminalEvent.type === "completed" ? terminalEvent.usage : undefined });
          if (viewedRef.current === current.item) { setPerformance({ state: "pending" }); if (terminalEvent.type === "completed") setUsage(terminalEvent.usage); }
          void readRequestPerformance(api, { instance_id: batch.runtime_instance_id, request_id: current.id, model_id: current.model_id, max_output_tokens: current.max_output_tokens, modality: "image", status: terminalEvent.type, ...(terminalEvent.type === "completed" ? { usage: terminalEvent.usage, finish_reason: terminalEvent.finish_reason } : {}) }).then(async (result) => {
            if (lifecycle.current === current.lifecycle) { updateItem(current.item, { performance: result }); if (viewedRef.current === current.item) setPerformance(result); }
            if (result.state === "ready" && batch.runtime_instance_id && result.record.error_code === savedRequest.error_code && await saved) {
              try {
                const success = await controller.persistOcrHistory({ ...savedRequest, mode: "update_performance", performance: { instance_id: batch.runtime_instance_id, record: result.record } });
                if (!success) notice("识别正文已保存，性能信息未能补存；当前结果仍保留。");
                changed();
              } catch (error) {
                if (!(error && typeof error === "object" && "code" in error && error.code === "ocr_history_not_found")) notice("识别正文已保存，性能信息未能补存；当前结果仍保留。");
              }
            }
          });
          const persisted = !savedRequest.markdown.trim() || await saved;
          if (current.overflow) setStatus("本张正文达到 256 KiB 上限，已停止并保留部分内容；请核对原图，批量已暂停。");
          advance = terminalEvent.type === "completed" && !current.hadReadFailure && !current.cancelled && persisted;
          if (!persisted && queueRef.current.length > 1) setStatus("本张正文未能保存，批量已暂停。请先复制或另存当前结果；不会自动重新识别。");
        }
      }
      // Only consuming this owned request's terminal releases the shared slot.
      task.current = null;
      void controller.refresh();
      return advance;
    } catch {
      current.hadReadFailure = true;
      current.cancelled = true;
      await api.chatCancel(current.id).catch(() => {});
      setStatus("识别结果与停止尚未确认，已保留任务。请重新确认识别任务；不会自动重新生成。");
      setRecovering(true);
    } finally { current.reading = false; current.readWaiters.forEach((resolve) => resolve()); current.readWaiters.clear(); }
  }, [api, controller, updateItem]);
  useEffect(() => controller.registerPreClose(async () => {
    const run = batchRun.current;
    if (run) run.stopped = true;
    const current = task.current;
    if (!current) { if (run) await run.done; return true; }
    current.cancelled = true;
    setStatus("关闭前正在停止识别并保存已有正文…");
    await current.started;
    if (current.id) await api.chatCancel(current.id).catch(() => {});
    if (current.reading) await new Promise<void>((resolve) => current.readWaiters.add(resolve));
    if (task.current === current && current.id) await consume(current);
    if (run) { await run.done; if (!task.current) controller.releaseOcrBatch(run.token); }
    return task.current !== current;
  }), [api, controller, consume]);
  const recover = async () => {
    const current = task.current;
    if (!current?.id || current.reading) return;
    setRecovering(false);
    setStatus("正在确认原识别任务…");
    // A failed cancel ACK must not prevent reading the authoritative terminal.
    await api.chatCancel(current.id).catch(() => {});
    await consume(current);
    if (!task.current) { if (batchRun.current) controller.releaseOcrBatch(batchRun.current.token); if (queueRef.current.length === 1) { frozen.current = null; setSettingsFrozen(false); } setBusy(false); }
  };
  const start = async (resume = false) => {
    const snapshot = controller.getSnapshot();
    if (snapshot.closing || !api.ocrStart || task.current || busy || pairOperation.current || blocked || !queueRef.current.length) return;
    const settings = resume ? frozen.current : { model, prompt, tokens, edge };
    if (!settings || !settings.prompt.trim() || !Number.isInteger(settings.tokens) || settings.tokens < 1 || settings.tokens > 4096) return;
    if (snapshot.snapshot?.runtime?.state !== "ready" || snapshot.snapshot.runtime.selected_model !== settings.model || (snapshot.snapshot.runtime.load_options?.context_size != null && settings.tokens >= snapshot.snapshot.runtime.load_options.context_size)) {
      setStatus("冻结的 OCR 模型当前未就绪，请加载原模型后再继续未开始图片。"); return;
    }
    if (!resume && queueRef.current.length === 1 && (!image || preparing || preparedGeneration.current !== imageGeneration.current)) return;
    const token = controller.acquireOcrBatch();
    if (!token) { setStatus("当前有其他任务，请等待后再开始识别。"); return; }
    if (!resume && queueRef.current.length === 1) updateItem(queueRef.current[0].id, { id: ++nextItem.current, status: "待开始", text: "", notice: "", performance: null, usage: undefined, incomplete: false });
    frozen.current = settings; setSettingsFrozen(true);
    const run: BatchRun = { ...settings, token, stopped: false, done: Promise.resolve() };
    batchRun.current = run; setBusy(true); setRecovering(false);
    run.done = (async () => {
      try {
        for (const item of queueRef.current.filter((item) => item.status === "待开始")) {
          if (run.stopped || controller.getSnapshot().closing || !controller.ownsOcrBatch(token)) break;
          viewedRef.current = item.id; setViewed(item.id); setText(""); setPerformance(null); setUsage(undefined); setHistoryNotice("");
          updateItem(item.id, { status: "准备图片" }); setPreparing(true); setPreviewItem(null); if (queueRef.current.length > 1) setImage(""); setImageError("");
          let prepared: string;
          try {
            prepared = !resume && queueRef.current.length === 1 ? image : await prepareOcrImage(item.file, settings.edge);
          } catch (error) {
            updateItem(item.id, { status: "失败" }); setStatus(error instanceof Error ? error.message : "图片准备失败，批量已暂停。"); break;
          } finally { setPreparing(false); }
          if (run.stopped || controller.getSnapshot().closing || !controller.ownsOcrBatch(token)) { updateItem(item.id, { status: "待开始" }); break; }
          await controller.refreshOcrBatch(token);
          const fresh = controller.getSnapshot().snapshot;
          if (run.stopped || controller.getSnapshot().closing || !controller.ownsOcrBatch(token)) { updateItem(item.id, { status: "待开始" }); break; }
          if (fresh?.connection !== "connected" || fresh.runtime?.state !== "ready" || fresh.runtime.selected_model !== settings.model || fresh.runtime.active_request || fresh.runtime.queued_jobs > 0 || fresh.runtime.stopping || fresh.runtime.registry_busy || (fresh.runtime.load_options?.context_size != null && settings.tokens >= fresh.runtime.load_options.context_size)) {
            updateItem(item.id, { status: "待开始" }); setStatus("服务或冻结的 OCR 模型状态已变化，批量已暂停；请恢复原模型后继续未开始图片。"); break;
          }
          setImage(prepared); setPreviewItem(item.id); if (queueRef.current.length > 1) setImageSelection(null); updateItem(item.id, { status: "识别中" });
          let markStarted!: () => void;
          const started = new Promise<void>((resolve) => { markStarted = resolve; });
          const current = { item: item.id, overflow: false, id: null as string | null, cancelled: false, reading: false, hadReadFailure: false, model_id: settings.model, max_output_tokens: settings.tokens, epoch: ++performanceEpoch.current, lifecycle: lifecycle.current, image_name: item.file.name, content: "", started, markStarted, readWaiters: new Set<() => void>() };
          task.current = current; setStatus("正在识别…");
          try {
            current.id = (await api.ocrStart!({ model_id: settings.model, image_data_url: prepared, prompt: settings.prompt, max_output_tokens: settings.tokens })).request_id;
            current.markStarted();
            if (!current.id) throw new Error("识别请求未返回有效标识。");
          } catch (error) {
            current.markStarted(); task.current = null; updateItem(item.id, { status: "失败" });
            setStatus(error instanceof Error ? error.message : "识别未能开始，批量已暂停。"); break;
          }
          if (current.cancelled || run.stopped) { current.cancelled = true; await api.chatCancel(current.id).catch(() => {}); }
          const advance = await consume(current);
          if (!advance || run.stopped) break;
        }
      } finally {
        if (!task.current) { controller.releaseOcrBatch(token); if (queueRef.current.length === 1) { frozen.current = null; setSettingsFrozen(false); } setBusy(false); }
        if (run.stopped && !task.current) setStatus((value) => value === "停止中…" ? "已停止，已生成结果保留；剩余图片未开始。" : value);
        setPreparing(false); void controller.refresh();
      }
    })();
    await run.done;
  };
  return <section className="ocr-page">
    <div className="page-heading"><div><span className="eyebrow">本机顺序识别</span><h1>图片 OCR</h1><p>选择图片，提取文字并保存为 Markdown。</p></div><span className="subtle-pill">{ready ? "OCR 模型已就绪" : "等待加载 OCR 模型"}</span></div>
    <section className="ocr-card ocr-model-card" aria-labelledby="ocr-model-heading">
      <div className="ocr-card-heading"><span className="ocr-step" aria-hidden="true">1</span><div><h2 id="ocr-model-heading">准备模型</h2><p>首次使用先导入两个配套文件，之后选择已导入的模型加载。</p></div></div>
      <fieldset aria-label="OCR 模型准备" disabled={busy || selecting || importing || blocked || !workbench.hydrated}>
        <div className="ocr-model-row">
          <label className="ocr-model-select">OCR 模型<select aria-label="OCR 模型" disabled={settingsFrozen} value={model} onChange={(event) => setModel(event.target.value)}><option value="">{models.length ? "选择模型" : "暂无 OCR 模型，请先在下方导入"}</option>{model && !selected && <option value={model}>{model} · 已保存选择（当前不可用）</option>}{models.map((item) => <option key={item.id} value={item.id}>{item.display_name} · 视觉配对</option>)}</select></label>
          <button disabled={!selected || !selected.available || !selected.loadable || !loadValid || profilePending || profileConflict || profileError} onClick={() => void controller.loadModel(model, { context_size: contextSize, batch_size: batchSize, threads })}>加载所选 OCR 模型</button>
        </div>
        <div className="ocr-parameter-grid">
          <label>本次上下文<input aria-label="OCR 加载上下文" type="number" min={32} value={contextSize} disabled={profilePending || settingsFrozen} onChange={(event) => setLoadOptions({ context_size: Number(event.target.value) })} /></label>
          <label>本次批次<input aria-label="OCR 加载批次" type="number" min={1} value={batchSize} disabled={profilePending || settingsFrozen} onChange={(event) => setLoadOptions({ batch_size: Number(event.target.value) })} /></label>
          <label>本次线程<input aria-label="OCR 加载线程" type="number" min={1} max={256} value={threads} disabled={profilePending || settingsFrozen} onChange={(event) => setLoadOptions({ threads: Number(event.target.value) })} /></label>
        </div>
        {profilePending && <p role="status">正在读取所选模型的已保存参数…</p>}
        {profileError && <p role="status" className="warning-text">无法读取模型参数，请重新选择模型或恢复连接后再加载，当前输入保留。</p>}
        {profileConflict && <div className="notice-band warning" role="alert"><div><strong>所选模型参数已变化</strong><p>本地 OCR 参数草稿与已保存档案都发生了修改。加载前请选择采用档案，或确认保留当前输入。</p><div className="workspace-actions"><button onClick={() => resolveConflict(false)}>采用已保存模型参数</button><button onClick={() => resolveConflict(true)}>保留当前 OCR 参数</button></div></div></div>}
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
        <div className="ocr-card-heading"><span className="ocr-step" aria-hidden="true">2</span><div><h2 id="ocr-image-heading">选择图片</h2><p>可多选图片，按队列顺序逐张识别。</p></div></div>
        <fieldset aria-label="OCR 图片与识别设置" disabled={busy || selecting || importing || blocked || !workbench.hydrated}>
          <div className="ocr-upload">
            <div><label htmlFor="ocr-image-input">图片文件</label><p id="ocr-image-format" className="small-note">PNG / JPEG，最多 4 MiB / 张；最多 20 张 / 合计 80 MiB</p></div>
            <button type="button" aria-describedby="ocr-image-format ocr-image-feedback" onClick={() => imageInput.current?.click()}>选择图片</button>
            <input ref={imageInput} id="ocr-image-input" hidden aria-label="OCR 图片" type="file" multiple accept=".png,.jpg,.jpeg,image/png,image/jpeg" aria-describedby="ocr-image-feedback" onChange={(event) => { chooseFiles(Array.from(event.target.files ?? [])); event.target.value = ""; }} />
          </div>
          <div className="ocr-image-preview">
            <div className="ocr-preview-frame">{image ? <img className="ocr-preview" src={image} alt="待识别图片预览" /> : <div className="ocr-empty-state"><strong>{preparing ? "正在准备图片…" : imageError ? "图片未能准备完成" : queue.length > 1 ? "按顺序逐张准备" : "图片预览"}</strong><p>{preparing ? "正在读取和检查图片，请稍候。" : imageError ? "请查看下方原因，重新选择图片后再试。" : queue.length > 1 ? "开始后仅准备当前图片，预览随识别进度更新。" : "选择图片后，在这里检查方向和文字清晰度。"}</p></div>}</div>
            <div id="ocr-image-feedback" className={`ocr-feedback ocr-image-feedback${imageError ? " warning-text" : ""}`} role="status" aria-label="OCR 图片准备状态">
              {(file || previewItem !== null) && <p className="ocr-selected-image">已选择：{file?.name ?? queue.find((item) => item.id === previewItem)?.file.name}</p>}
              <p>{imageError || (preparing ? edge ? "正在检查并准备所选尺寸的图片…" : "正在检查原图…" : image ? edge ? "图片已准备完成，预览为本次将发送的图片。" : "原图已准备完成，可在预览中核对文字清晰度。" : queue.length > 1 ? `已导入 ${queue.length} 张图片，将按队列序号逐张准备。` : "尚未选择图片。") }</p>
            </div>
          </div>
          <div className="ocr-image-settings">
            <label>发送图片尺寸<select aria-label="发送图片尺寸" value={edge} disabled={settingsFrozen} onChange={(event) => selectImage(file, Number(event.target.value))}><option value={0}>原图（不缩放）</option><option value={1600}>最长边 1600</option><option value={2048}>最长边 2048</option></select></label>
            <label>最大输出 token<input aria-label="最大输出 token" disabled={settingsFrozen} type="number" min={1} max={4096} value={tokens} onChange={(event) => controller.setOcrPreferences({ max_output_tokens: Number(event.target.value) })} /></label>
          </div>
          <p className="small-note">缩小图片可能丢失小字；输出达到上限时会提示内容可能截断。</p>
          <label className="ocr-prompt">识别提示词<textarea disabled={settingsFrozen} value={prompt} rows={2} maxLength={4096} onChange={(event) => controller.setOcrPreferences({ prompt: event.target.value })} /></label>
        </fieldset>
        {queue.length > 0 && <section className="ocr-queue" aria-label="OCR 图片队列"><h3>图片队列 · {queue.length} 张</h3><p className="small-note">已结束 {queue.filter((item) => ["已完成", "已停止", "失败"].includes(item.status)).length} / {queue.length}</p><p className="small-note">默认按导入顺序（以文件对话框返回列表与此处序号为准）。开始后冻结顺序与参数；失败或停止会暂停，继续仅处理未开始图片。原图仅留在当前窗口，结果逐张保存到本机历史。</p><ol>{queue.map((item, index) => <li key={item.id} aria-label={`队列图片 ${index + 1}：${item.file.name}`}><div><strong>{index + 1}. {item.file.name}</strong><span>{item.status}{item.incomplete ? " · 内容可能不完整" : ""}</span></div><div className="ocr-queue-actions"><button disabled={settingsFrozen || index === 0} aria-label={`上移 ${item.file.name}`} onClick={() => editQueue(item.id, -1)}>上移</button><button disabled={settingsFrozen || index === queue.length - 1} aria-label={`下移 ${item.file.name}`} onClick={() => editQueue(item.id, 1)}>下移</button><button disabled={settingsFrozen} aria-label={`移除 ${item.file.name}`} onClick={() => editQueue(item.id, 0)}>移除</button><button aria-label={`查看结果 ${item.file.name}`} aria-pressed={viewed === item.id} onClick={() => viewResult(item)}>查看结果</button></div></li>)}</ol></section>}
        <div className="ocr-run-actions"><button className="primary" disabled={settingsFrozen || !ready || !queue.length || (queue.length === 1 && !image) || busy || selecting || importing || preparing || blocked || !prompt.trim() || tokens < 1 || tokens > 4096 || !Number.isInteger(tokens) || !outputFits} onClick={() => void start()}>{queue.length > 1 ? "开始批量识别" : "识别图片"}</button>{settingsFrozen && queue.some((item) => item.status === "待开始") && <button disabled={busy || blocked} onClick={() => void start(true)}>继续未开始图片</button>}<button disabled={!busy} onClick={() => void stop()}>停止识别</button>{recovering && <button onClick={() => void recover()}>重新确认识别任务</button>}</div>
        <p className="ocr-feedback" role="status">{status || (!ready ? "请先加载所选 OCR 模型。" : preparing ? "图片正在准备，请稍候。" : imageError ? "图片准备失败，请重新选择图片。" : image ? "模型和图片已就绪，可开始识别。" : queue.length > 1 ? "图片队列已就绪，可开始批量识别。" : "模型已加载，请先选择图片。") }</p>
        <p className="small-note">当前加载上下文：{runtime?.selected_model === model ? runtime?.load_options?.context_size ?? "未知" : "未加载"}。图片、提示词和输出共同占用上下文。</p>
      </section>
      <section className="ocr-card ocr-output-card" aria-labelledby="ocr-output-heading">
        <div className="ocr-card-heading"><span className="ocr-step" aria-hidden="true">3</span><div><h2 id="ocr-output-heading">识别结果</h2><p>文字会逐步显示，完成后请对照原图核对。</p></div></div>
        {queue.find((item) => item.id === viewed) && <p className="ocr-result-filename">{queue.find((item) => item.id === viewed)?.file.name}</p>}
        <div className="ocr-result-toolbar">
          <div className="ocr-view-switch" role="group" aria-label="结果显示方式"><button aria-pressed={!markdown} onClick={() => controller.setOcrPreferences({ markdown: false })}>原文</button><button aria-pressed={markdown} onClick={() => controller.setOcrPreferences({ markdown: true })}>Markdown</button></div>
          <div className="ocr-export-actions"><button disabled={!text} onClick={() => void navigator.clipboard.writeText(text).then(() => setStatus("已复制原文。")).catch(() => setStatus("复制失败，请手动复制原文。"))}>复制识别原文</button><button disabled={!text || !api.saveOcrMarkdown} onClick={() => void api.saveOcrMarkdown?.(text).then((result) => setStatus(result.saved ? "已保存 Markdown 原文。" : "已取消保存。")).catch(() => setStatus("保存失败，原文仍保留。"))}>保存 .md</button></div>
        </div>
        <div className="ocr-result-body" tabIndex={0} role="region" aria-label="识别结果内容">{text ? markdown ? <ChatMarkdown content={text} /> : <pre className="ocr-result">{text}</pre> : <div className="ocr-empty-state"><strong>识别结果会显示在这里</strong><p>加载模型并选择图片后，点击“识别图片”。</p></div>}</div>
        {performance && <PerformanceSummary value={performance} usage={usage} image />}
        {historyNotice && <p className="ocr-history-save-status" role="status" aria-label="识别历史保存状态">{historyNotice}</p>}
      </section>
    </div>
    <OcrHistory api={api} changed={historyChanged} removeRecord={controller.deleteOcrHistory} closing={state.closing} />
  </section>;
}
