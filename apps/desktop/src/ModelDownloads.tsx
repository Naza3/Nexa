import { useEffect, useState } from "react";
import type { DesktopController, ViewState } from "./controller";
import type { DownloadSource } from "./types";

const sourceName = (source: DownloadSource) => source === "modelscope" ? "ModelScope" : "Hugging Face";
const fallbackDownloadMessage = "下载未完成，请提供诊断码和当前进度以便排查。";
function DownloadError({ error }: { error: unknown }) {
  // The bridge produces controlled messages, never raw network error strings.
  // Also fail closed for malformed display data without rendering objects or HTML.
  const value = error && typeof error === "object" ? error as Record<string, unknown> : {};
  const code = typeof value.code === "string" && /^[a-z0-9_]{1,80}$/.test(value.code) ? value.code : "invalid_download_error";
  const message = typeof value.message === "string" && value.message.trim() &&
    new TextEncoder().encode(value.message).byteLength <= 500 &&
    !Array.from(value.message).some((character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127) &&
    !/[<>\\]|:\/\/|www\.|\b(?:bearer|authorization|cookie|token|password|secret|signature|credential)\b/i.test(value.message)
    ? value.message : fallbackDownloadMessage;
  return <><p>{message}</p><p>诊断码：{code}</p></>;
}
function displayDirectory(path: unknown) {
  return typeof path === "string" && path.trim() && path.length <= 32768 && !path.includes("\0") ? path : "路径信息不可用";
}
function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KiB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GiB`;
}
export function DownloadProgress({ state, controller }: { state: ViewState; controller: DesktopController }) {
  const task = state.download;
  const active = state.download_phase !== "idle";
  if (!task && !active) return null;
  const phase = {
    connecting: "连接下载源", downloading: "下载文件", verifying: "校验文件", committing: "保存文件", finished: "已结束",
  } as const;
  const status = active ? state.download_phase === "recovery" ? "下载状态待确认" : state.download_phase === "stopping" ? "正在取消下载" : task ? phase[task.phase] : "正在提交下载" :
    task?.status === "completed" ? "文件已保存，尚未登记" : task?.status === "cancelled" ? "下载已取消" : "下载失败";
  return <section className="library-progress download-progress" aria-label="模型下载进度">
    <div role="status"><strong>{status}</strong>
      {task && <>
        <p>{task.file_name} · {sourceName(task.source)}</p>
        <p className="directory-path">保存到：{displayDirectory(task.target_display_path)}</p>
        <p>第{task.attempt ?? 1}次传输尝试</p>
        <p>已接收 {size(task.downloaded_bytes)}{task.total_bytes !== null ? ` / ${size(task.total_bytes)}` : " · 总大小未知"}</p>
        {task.total_bytes !== null && task.total_bytes > 0 && <progress aria-label="模型文件已接收字节" max={task.total_bytes} value={task.downloaded_bytes} />}
        {task.phase === "verifying" && <p>网络接收已结束，正在核验真实文件；尚未保存完成。</p>}
        {task.error !== null && <DownloadError error={task.error} />}
        {task.status === "completed" && state.snapshot?.model_directory.configured?.directory_id !== task.directory_id && <p>当前目录已变化。请回到上方保存目标后扫描登记。</p>}
        {task.result?.cleanup_warning && <p className="warning-text">文件已保存，部分下载文件清理未确认，请勿重复下载。{task.result.cleanup_warning}</p>}
      </>}
    </div>
    {state.download_phase === "recovery" ? <button onClick={() => void controller.recoverDownload()}>重新确认下载状态</button> : active ?
      <button disabled={state.download_phase === "stopping"} onClick={() => void controller.cancelDownload()}>{state.download_phase === "stopping" ? "正在取消…" : "取消下载"}</button> :
      task?.status === "completed" ? <button disabled={state.snapshot?.connection !== "stopped" || state.snapshot.model_directory.configured?.directory_id !== task.directory_id || state.library_phase !== "idle" || !!state.operation} onClick={() => void controller.scanModels()}>扫描目录以登记</button> :
        task && <button disabled={state.snapshot?.connection !== "stopped" || state.library_phase !== "idle" || !!state.operation} onClick={() => void controller.startDownload(task.catalog_id)}>重新下载</button>}
  </section>;
}
export function ModelDownloads({ state, controller, goSettings }: { state: ViewState; controller: DesktopController; goSettings: () => void }) {
  const [filter, setFilter] = useState("");
  useEffect(() => { void controller.loadCatalog(); }, [controller]);
  const source = state.snapshot?.settings.download_source ?? "modelscope";
  const directory = state.snapshot?.model_directory.configured;
  const stopped = state.snapshot?.connection === "stopped";
  const busy = state.download_phase !== "idle" || state.library_phase !== "idle" || !!state.operation;
  const query = filter.trim().toLocaleLowerCase();
  const entries = state.catalog.filter((entry) => [entry.display_name, entry.file_name, entry.architecture, entry.quantization].some((value) => value.toLocaleLowerCase().includes(query)));
  return <section className="download-catalog" aria-labelledby="catalog-title">
    <div className="section-heading"><div><h2 id="catalog-title">下载 GGUF 模型</h2><p>精选下载目录不是加载白名单，仍可使用目录外的本地兼容 GGUF。</p></div><button disabled={state.catalog_loading} onClick={() => void controller.loadCatalog()}>刷新下载目录</button></div>
    <div className="directory-summary"><div><strong>已保存下载源：{sourceName(source)}</strong><p className="directory-path">下载到：{directory?.display_path ?? "尚未选择模型目录"}</p><p>在设置中更改来源并保存后生效；不会自动切换到其他下载源。</p></div><button onClick={goSettings}>目录与下载源设置</button></div>
    {!directory && <p role="status">{state.discovery === "checking" ? "正在自动发现程序旁的 models 目录…" : state.discovery === "none" ? "未发现程序旁的 models 目录，请先选择并应用模型目录。" : "请先完成模型目录登记；已保存目录始终优先。"}</p>}
    {!stopped && <p className="warning-text">下载和登记前须显式停止运行服务；不会自动停止其他客户端。</p>}
    <p className="warning-text">未在你的目标机实测。文件大小不等于运行内存；16GB 总内存需与系统、模型和上下文共享，大模型或长上下文可能加载失败。这里不保证推理速度或可用内存。</p>
    <label className="catalog-filter">筛选下载目录<input type="search" value={filter} onChange={(event) => setFilter(event.target.value)} placeholder="模型名、架构或量化" /></label>
    {state.catalog_loading && <p role="status">正在读取下载目录…</p>}
    {state.catalog_loaded && entries.length === 0 && <p role="status">{query ? "没有匹配的模型，请更改筛选词。" : "当前下载目录没有条目。"}</p>}
    <div className="model-list">{entries.map((entry) => {
      const chosen = entry.sources.find((item) => item.source === source);
      return <article className="model-row catalog-row" key={entry.catalog_id}>
        <div className="model-description"><h3>{entry.display_name}</h3><p className="catalog-filename">{entry.file_name}</p><div className="model-meta"><span>{entry.quantization || "量化未知"}</span><span>{size(entry.size_bytes)}</span><span>{entry.architecture || "架构未知"}</span><span>未在目标机实测</span></div>
          <p>{entry.recommendation}</p><p>许可：{entry.license || "未提供"} · 上下文参考：{entry.context_hint} tokens（非内存保证）</p>
          <details><summary>来源与文件身份</summary><p>SHA-256：{entry.sha256}</p>{entry.sources.map((item) => <p key={item.source}>{sourceName(item.source)}：{item.repository}<br />版本：{item.revision}<br />地址：{item.url}</p>)}</details>
          {!chosen && <p className="warning-text">{sourceName(source)} 暂无此文件；可前往设置选择可用来源，不会自动回退。</p>}
        </div>
        <button className="primary" disabled={busy || !stopped || !directory || !chosen} aria-label={`下载 ${entry.display_name}`} onClick={() => void controller.startDownload(entry.catalog_id)}>下载</button>
      </article>;
    })}</div>
  </section>;
}
