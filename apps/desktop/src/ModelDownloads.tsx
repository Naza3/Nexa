import { useEffect, useState } from "react";
import type { DesktopController, ViewState } from "./controller";
import type { DownloadSource } from "./types";
import { LocalValidationFeedback } from "./ModelTestFeedback";
import { DetailsGroup } from "./DetailsGroup";

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
  const saved = task?.phase === "registering" || task?.phase === "testing";
  if (!task && !active) return null;
  const phase = {
    connecting: "连接下载源", downloading: "下载文件", verifying: "校验文件", committing: "保存文件", registering: "文件已保存 · 正在登记", testing: "文件已登记 · 正在加载与基础测试", finished: "已结束",
  } as const;
  const status = active ? state.download_phase === "recovery" ? "下载状态待确认" : state.download_phase === "stopping" ? saved ? "正在取消后续步骤，已保存文件保留" : "正在取消下载" : task ? phase[task.phase] : "正在提交下载" :
    task?.status === "completed" ? task.result?.registered ? "文件已保存并登记" : "文件已保存，尚未登记" : task?.status === "cancelled" ? "下载已取消" : "下载失败";
  return <section className="library-progress download-progress" aria-label="模型下载进度">
    <div role="status"><strong>{status}</strong>
      {task && <>
        <p>{task.file_name} · {sourceName(task.source)}</p>
        <p className="directory-path">保存到：{displayDirectory(task.target_display_path)}</p>
        <p>第{task.attempt ?? 1}次传输尝试</p>
        <p>已接收 {size(task.downloaded_bytes)}{task.total_bytes !== null ? ` / ${size(task.total_bytes)}` : " · 总大小未知"}</p>
        {task.total_bytes !== null && task.total_bytes > 0 && <progress aria-label="模型文件已接收字节" max={task.total_bytes} value={task.downloaded_bytes} />}
        {task.phase === "verifying" && <p>网络接收已结束，正在核验真实文件；尚未保存完成。</p>}
        {task.phase === "registering" && <p>下载文件已保存，正在更新模型索引；登记失败不会把已保存文件显示为下载失败。</p>}
        {task.phase === "testing" && <p>正在执行已开启的自动加载与短文本测试。加载最多 300 秒，短测最多 30 秒；若服务忙碌或已加载其他模型，将暂缓，不会切换模型或中断其他客户端。</p>}
        {task.error !== null && <DownloadError error={task.error} />}
        {task.result?.registration_error && <div className="registration-error"><strong>自动登记未完成，已保存文件仍保留</strong><DownloadError error={task.result.registration_error} /></div>}
        {task.result?.local_validation && <div className="download-validation"><LocalValidationFeedback value={task.result.local_validation} /><p className="small-note">仅证明此文件与当前引擎、设备、加载参数的加载及短文本生成；不证明回答质量、长上下文或工具调用能力。</p></div>}
        {task.status === "completed" && state.snapshot?.model_directory.configured?.directory_id !== task.directory_id && <p>下载目录已变化，已保存文件仍在原下载位置。可使用“添加模型”选择该文件登记。</p>}
        {task.result?.cleanup_warning && <p className="warning-text">文件已保存，部分下载文件清理未确认，请勿重复下载。{task.result.cleanup_warning}</p>}
      </>}
    </div>
    {state.download_phase === "recovery" ? <button onClick={() => void controller.recoverDownload()}>重新确认下载状态</button> : active ?
      <button disabled={state.download_phase === "stopping"} onClick={() => void controller.cancelDownload()}>{state.download_phase === "stopping" ? "正在取消…" : saved ? "取消后续步骤" : "取消下载"}</button> :
      task?.status === "completed" ? !task.result?.registered && <button disabled={state.library_phase !== "idle" || !!state.operation} onClick={() => void controller.pickModels()}>选择已保存文件以登记</button> :
        task && <button disabled={state.snapshot?.connection !== "stopped" || state.library_phase !== "idle" || !!state.operation} onClick={() => void controller.startDownload(task.catalog_id, state.download_auto_test)}>重新下载</button>}
    {!active && task?.terminal && <button onClick={() => controller.dismissDownloadResult(task)}>关闭下载结果</button>}
  </section>;
}
export function ModelDownloads({ state, controller, goSettings, initialFilter = "", onFilterChange }: { state: ViewState; controller: DesktopController; goSettings: () => void; initialFilter?: string; onFilterChange?: (value: string) => void }) {
  const [filter, setFilter] = useState(initialFilter);
  const [autoTest, setAutoTest] = useState(false);
  useEffect(() => { void controller.loadCatalog(); }, [controller]);
  const source = state.snapshot?.settings.download_source ?? "modelscope";
  const directory = state.snapshot?.model_directory.configured;
  const stopped = state.snapshot?.connection === "stopped";
  const busy = state.download_phase !== "idle" || state.library_phase !== "idle" || !!state.operation;
  const query = filter.trim().toLocaleLowerCase();
  const entries = state.catalog.filter((entry) => [entry.display_name, entry.file_name, entry.architecture, entry.quantization].some((value) => value.toLocaleLowerCase().includes(query)));
  return <section className="download-catalog" aria-labelledby="catalog-title">
    <div className="section-heading"><div><h2 id="catalog-title">下载 GGUF 模型</h2></div><button disabled={state.catalog_loading} onClick={() => void controller.loadCatalog()}>刷新下载目录</button></div>
    <div className="download-source-row"><div><span>下载源：{sourceName(source)}</span>{directory && <p className="download-target" title={directory.display_path}>下载到：{directory.display_path}</p>}</div><button className="text-button" onClick={goSettings}>目录与下载源设置</button></div>
    {!directory && <p role="status">请在设置中选择下载目录。已有 GGUF 可直接用“添加模型”登记，无需设置目录。</p>}
    {!stopped && <p className="warning-text">下载和登记前须显式停止运行服务；不会自动停止其他客户端。</p>}
    <label className="auto-test-option"><input type="checkbox" checked={autoTest} disabled={busy} onChange={(event) => setAutoTest(event.target.checked)} />下载后加载并进行基础测试（空闲时）</label>
    <DetailsGroup title="下载与测试说明"><p className="small-note">文件完成校验后会自动登记。启用此选项会启动本机服务，只尝试本次下载的模型；若已有模型或任务则暂缓。取消勾选后仅保存和登记。</p>
    <p className="warning-text">未在你的目标机实测。文件大小不等于运行内存；16GB 总内存需与系统、模型和上下文共享，大模型或长上下文可能加载失败。这里不保证推理速度或可用内存。</p><p>精选下载目录不是加载白名单，仍可使用目录外的本地兼容 GGUF。</p></DetailsGroup>
    <label className="catalog-filter">筛选下载目录<input type="search" value={filter} onChange={(event) => { setFilter(event.target.value); onFilterChange?.(event.target.value); }} placeholder="模型名、架构或量化" /></label>
    {state.catalog_loading && <p role="status">正在读取下载目录…</p>}
    {state.catalog_loaded && entries.length === 0 && <p role="status">{query ? "没有匹配的模型，请更改筛选词。" : "当前下载目录没有条目。"}</p>}
    <div className="model-list">{entries.map((entry) => {
      const chosen = entry.sources.find((item) => item.source === source);
      return <article className="model-row catalog-row" key={entry.catalog_id}>
        <div className="model-description"><h3>{entry.display_name}</h3><div className="model-meta"><span>{entry.quantization || "量化未知"}</span><span>{size(entry.size_bytes)}</span><span>{entry.architecture || "架构未知"}</span><span>未在目标机实测</span></div>
          <details><summary>模型详情与下载来源</summary><p className="catalog-filename">{entry.file_name}</p><p>{entry.recommendation}</p><p>许可：{entry.license || "未提供"} · 上下文参考：{entry.context_hint} tokens（非内存保证）</p>
          <div><p>SHA-256：{entry.sha256}</p>{entry.sources.map((item) => <p key={item.source}>{sourceName(item.source)}：{item.repository}<br />版本：{item.revision}<br />地址：{item.url}</p>)}</div></details>
          {!chosen && <p className="warning-text">{sourceName(source)} 暂无此文件；可前往设置选择可用来源，不会自动回退。</p>}
        </div>
        <button className="primary" disabled={busy || !stopped || !directory || !chosen} aria-label={`下载 ${entry.display_name}`} onClick={() => void controller.startDownload(entry.catalog_id, autoTest)}>下载</button>
      </article>;
    })}</div>
  </section>;
}
