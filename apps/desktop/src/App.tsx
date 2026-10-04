import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { FormEvent, ReactNode } from "react";
import {
  byteLength,
  DEFAULT_SETTINGS,
  DesktopController,
  LIMITS,
  validatePreferences,
} from "./controller";
import type { ViewState } from "./controller";
import type { Preferences, RuntimeStatus, Settings } from "./types";
import { ModelDownloads, DownloadProgress } from "./ModelDownloads";
import { ModelSelectionPanel, AddModelProgress } from "./ModelAdd";
import { modelCompatibility } from "./modelCompatibility";
import { localValidationReason } from "./localValidation";
import { LocalValidationFeedback, ModelTestFeedback } from "./ModelTestFeedback";
import { Modal } from "./Modal";
import { GlobalConfiguration, RequestDefaultsForm, UiPreferencesForm, ModelProfile } from "./Configuration";
import { ownRecord } from "./records";
import { OverviewPage, ApiPage, ActivityPage } from "./Workspaces";
import { runtimeView } from "./runtimeView";
import { useConfigDraft, ConfigDraftContext, createDraftStore, hasDirtyDrafts } from "./configDraft";
import { preferencesOnly } from "./runtimeSettingsValues";
import { IdleUnloadSettings, VerificationTimeoutSettings } from "./RuntimeSettings";

function Icon({ name, size = 20 }: { name: string; size?: number }) {
  const paths: Record<string, ReactNode> = {
    models: (
      <>
        <path d="m12 3 8 4.5v9L12 21l-8-4.5v-9L12 3Z" />
        <path d="m4 7.5 8 4.5 8-4.5M12 12v9M8 5.3l8 4.5" />
      </>
    ),
    chat: (
      <path d="M20 11.5a7.5 7.5 0 0 1-7.5 7.5H6l-3 2V11.5A7.5 7.5 0 0 1 10.5 4h2a7.5 7.5 0 0 1 7.5 7.5Z" />
    ),
    settings: (
      <>
        <path d="M4 7h16M4 17h16" />
        <circle cx="9" cy="7" r="3" />
        <circle cx="15" cy="17" r="3" />
      </>
    ),
    plus: <path d="M12 5v14M5 12h14" />,
    arrow: <path d="m5 12 7-7 7 7M12 5v14" />,
    file: (
      <>
        <path d="M14 3H6v18h12V7l-4-4Z" />
        <path d="M14 3v5h4M9 13h6M9 17h6" />
      </>
    ),
    shield: (
      <>
        <path d="m12 3 8 3v6c0 4-5 8-8 9-3-1-8-5-8-9V6l8-3Z" />
        <path d="m8 12 3 3 5-6" />
      </>
    ),
    stop: <rect x="6" y="6" width="12" height="12" rx="2" />,
    refresh: (
      <>
        <path d="M20 8a8 8 0 1 0 .5 7M20 3v5h-5" />
      </>
    ),
    close: <path d="m6 6 12 12M6 18 18 6" />,
    copy: (
      <>
        <rect x="8" y="8" width="12" height="13" rx="2" />
        <path d="M16 8V3H3v13h5" />
      </>
    ),
    chevron: <path d="m9 5 7 7-7 7" />,
    power: (
      <>
        <path d="M12 2v10M6 5a9 9 0 1 0 12 0" />
      </>
    ),
  };
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {paths[name] ?? paths.models}
    </svg>
  );
}
function Brand({ small = false }: { small?: boolean }) {
  return (
    <div className={`brand-mark ${small ? "small" : ""}`} aria-hidden="true">
      <span />
      <span />
      <span />
    </div>
  );
}
function Spinner() {
  return <span className="spinner" aria-hidden="true" />;
}
const stateNames: Record<RuntimeStatus["state"], string> = {
  unloaded: "未加载模型",
  loading: "正在加载",
  ready: "模型就绪",
  generating: "正在生成",
  unloading: "正在卸载",
  faulted: "模型运行故障",
};
function statusLabel(state: ViewState) {
  return state.booting ? "正在确认服务" : runtimeView(state.snapshot).serviceLabel;
}
function formatSize(bytes: number) {
  return bytes >= 1024 ** 3
    ? `${(bytes / 1024 ** 3).toFixed(2)} GiB`
    : `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
}
function RuntimeBanner({ state }: { state: ViewState }) {
  const snapshot = state.snapshot;
  if (state.booting) return <div className="notice-band"><Spinner />正在检查本机运行服务…</div>;
  if (!snapshot) return <div className="notice-band warning"><div><strong>尚未连接到桌面服务</strong><p>请在 Nexa 桌面应用中打开，或使用左侧按钮重新检查服务。</p></div></div>;
  if (snapshot.connection === "error") return <div className="notice-band warning"><div><strong>与运行服务的连接失效</strong><p>请使用左侧按钮重新检查服务。无法验证的现有实例不会被替换，请检查后重试。</p></div></div>;
  if (!snapshot.initialized) return <div className="notice-band"><div><strong>第一次使用 Nexa</strong><p>可先在设置中“仅初始化配置”，再调整运行选项；也可点击左侧“初始化并启动”。已有凭据不会被轮换。</p></div></div>;
  if (snapshot.connection !== "connected") return <div className="notice-band"><div><strong>运行服务尚未就绪</strong><p>{snapshot.connection === "connecting" ? "正在连接运行服务，请等待状态确认。" : "可直接浏览已登记的本地模型。点击左侧按钮启动服务；点击加载也会启动本机服务，并进行基础测试。"}</p></div></div>;
  if (snapshot.runtime?.state === "faulted") return <div className="notice-band warning"><div><strong>模型运行故障</strong><p>{snapshot.runtime.last_error?.message ?? "请在模型页显式重新加载，恢复后再发送。已有请求不会重放。"}</p></div></div>;
  return null;
}
function ServiceControl({ state, controller, onStop }: { state: ViewState; controller: DesktopController; onStop: () => void }) {
  const snapshot = state.snapshot;
  const operation = state.operation;
  const starting = operation?.kind === "start";
  const stopping = operation?.kind === "stop" || !!snapshot?.runtime?.stopping;
  const checking = operation?.kind === "check";
  const stopped = snapshot?.connection === "stopped";
  const connected = snapshot?.connection === "connected";
  const connecting = snapshot?.connection === "connecting";
  const busy = !!operation || state.library_phase !== "idle" || state.download_phase !== "idle" || state.chat_phase !== "idle";
  const pending = state.booting || starting || stopping || checking || connecting;
  const label = state.booting || checking ? "正在检查服务…" : stopping ? "正在停止服务…" : starting || connecting ? "正在启动服务…"
    : stopped ? snapshot.initialized ? "启动运行服务" : "初始化并启动" : connected ? "停止运行服务" : "重新检查服务";
  const status = state.booting || checking ? "正在确认状态" : stopping ? "正在停止，等待清理确认" : starting || connecting ? "正在启动，等待连接"
    : stopped ? "服务已停止" : connected ? snapshot.runtime?.state === "faulted" ? "服务运行中 · 模型故障" : "服务运行中" : "服务状态待确认";
  return <section className="service-control" aria-label="运行服务控制">
    <div className={`service-control-status ${connected && !stopping ? "running" : ""}`} role="status">
      {pending ? <Spinner /> : <span className="status-dot" />}<span>{status}</span>
    </div>
    <button className={connected ? "danger-outline service-main-button" : "primary service-main-button"} disabled={pending || busy}
      onClick={() => { if (stopped) void controller.start(!snapshot.initialized); else if (connected) onStop(); else void controller.checkService(); }}>
      {pending ? <Spinner /> : <Icon name={connected ? "stop" : stopped ? "power" : "refresh"} size={17} />}{label}
    </button>
    <p>{busy && !pending ? "请先完成或取消当前操作" : connected ? "停止会中断其他应用的调用" : stopped ? "启动后可供其他应用调用" : "确认状态后再启动或停止"}</p>
  </section>;
}
function DirectoryStateNotice({ state }: { state: ViewState }) {
  const directory = state.snapshot?.model_directory;
  if (!directory) return null;
  const explanations = {
    default: "未设置默认下载目录，已登记模型仍可使用。",
    ready: "当前服务已采用已保存的默认目录配置；各模型仍按登记来源读取。",
    stopped: "已保存目录，可直接浏览已登记的模型；浏览不会启动运行服务。",
    stale:
      "已保存目录与运行实例不一致。请显式停止服务，再启动匹配配置；不会自动切换模型。",
    missing:
      "已保存的默认目录已失踪。请检查路径或在停止服务后重新选择；其他来源模型以各自可用状态为准。",
    unavailable:
      "无法读取已保存的模型目录。请检查本地目录权限；目录仅需可读，无需可写。",
    unsupported: "当前运行服务不支持所选目录，请先停止，再启动匹配版本。",
  };
  return (
    <div
      className={`directory-state ${["stale", "missing", "unavailable", "unsupported"].includes(directory.state) ? "warning-text" : ""}`}
      role="status"
    >
      <p>{explanations[directory.state]}</p>
      {directory.effective && (
        <p>
          当前服务默认目录：
          <span className="directory-path">
            {directory.effective.display_path}
          </span>
        </p>
      )}
    </div>
  );
}
function LibraryProgress({
  state,
  controller,
}: {
  state: ViewState;
  controller: DesktopController;
}) {
  if (state.library_kind === "add" || state.library_phase === "idle") return null;
  const phases = {
    checking: "检查停止状态与目录",
    enumerating: "枚举目录条目",
    verifying: "核验 GGUF 文件",
    committing: "保存模型库索引",
    testing: "正在进行基础测试",
    finished: "操作已结束",
  };
  const progress = state.library;
  return (
    <section className="library-progress" aria-label="模型库操作">
      <div>
        <div className="progress-heading">
          <Spinner />
          <strong>
            {state.library_phase === "recovery"
              ? "模型库操作终态尚未确认"
              : state.library_phase === "stopping"
                ? "正在取消，等待实际操作结束"
                : state.library_kind === "configure"
                ? "正在保存默认下载目录"
                : progress
                  ? phases[progress.phase]
                  : "正在提交模型库操作"}
          </strong>
        </div>
        {progress && state.library_kind !== "configure" && (
          <p>
            已检查 {progress.examined_entries} 个目录条目 · 发现{" "}
            {progress.candidate_files} 个 GGUF · 已核验{" "}
            {progress.verified_files} 个文件
          </p>
        )}
        <p>
          {state.library_phase === "recovery"
            ? "不会自动重做扫描或宣称保存成功，请重新确认终态。"
            : state.library_kind === "configure" ? "仅保存默认下载目录，不扫描文件；已有模型索引保持不变。" : "模型文件只读核验，不复制；取消需等待实际终态，停止确认不代表已经结束。"}
        </p>
      </div>
      {state.library_phase === "recovery" ? (
        <button onClick={() => void controller.recoverLibrary()}>
          重新确认模型库操作
        </button>
      ) : (
        <button
          disabled={state.library_phase === "stopping" && !state.error}
          onClick={() => void controller.cancelLibrary()}
        >
          {state.library_phase === "stopping"
            ? "等待取消确认"
            : "取消模型库操作"}
        </button>
      )}
    </section>
  );
}
function LibraryDiagnostics({ state }: { state: ViewState }) {
  const operation = state.library;
  if (state.library_kind === "add" || !operation?.terminal || !(operation.file_errors?.length)) return null;
  const partial = operation.status === "partial";
  const uncertain = operation.error?.code === "settings_durability_unconfirmed";
  return <section className="notice-band warning library-diagnostics" aria-label="模型目录核验结果" role="status">
    <div>
      <strong>{partial ? `目录已部分登记：${operation.result!.registered_files} 个已登记，${operation.file_errors.length} 个未登记` : "本次核验发现未通过的文件"}</strong>
      <p>{partial ? "合法文件已一次保存；下列文件未登记。可在本地列表查看；可尝试加载不代表已实测。" : uncertain ? "索引可能已保存，持久化尚未确认；请先核对实际配置，不能假定已回滚。" : "本次未发布新索引，原目录与索引保留。下列原因仅表示本次已检查的文件。"}</p>
      <ul>{operation.file_errors.map((failure) => <li key={failure.file_name}>
        <strong>{failure.file_name}</strong>：{failure.code === "invalid_manifest" ? "GGUF 结构或必需元数据无效，或超出当前结构支持范围" : failure.code === "unsupported_model" ? "当前仅支持受保护的单文件文本 GGUF，分片不受支持" : "缺少有效的原始嵌入聊天模板，不使用替代模板"}
        <span className="subtle">（{failure.code}）</span>
      </li>)}</ul>
      <p>源文件未被移动、删除或改写。修复后请显式重新扫描；这些诊断仅保留在当前窗口中。</p>
    </div>
  </section>;
}
function DirectorySettings({
  state,
  controller,
}: {
  state: ViewState;
  controller: DesktopController;
}) {
  const [confirm, setConfirm] = useState<"apply" | "scan" | "stop" | null>(
    null,
  );
  const directory = state.snapshot!.model_directory;
  const stopped = state.snapshot?.connection === "stopped";
  const running =
    state.snapshot?.connection === "connected" && !state.snapshot.runtime?.stopping;
  const busy =
    !!state.operation ||
    state.library_phase !== "idle" ||
    state.download_phase !== "idle" ||
    state.chat_phase !== "idle";
  return (
    <section
      className="settings-card directory-settings"
      aria-labelledby="directory-title"
    >
      <div className="card-heading">
        <div>
          <h2 id="directory-title">下载目录与手动维护</h2>
          <p>
            下载文件保存在此目录。添加已有模型请使用模型页“添加模型”，无需更改此目录。
          </p>
        </div>
        <span className="subtle-pill">只读文件</span>
      </div>
      <div className="directory-value">
        <span>
          {state.library?.error?.code === "settings_durability_unconfirmed" &&
          state.snapshot?.connection === "error"
            ? "上次读取目录（当前配置尚未确认）"
            : "已保存目录"}
        </span>
        <output className="directory-path">
          {directory.configured?.display_path ?? "未选择"}
        </output>
      </div>
      <DirectoryStateNotice state={state} />
      <div className="directory-actions">
        <button disabled={busy} onClick={() => void controller.pickDirectory()}>
          <Icon name="file" size={16} />
          选择模型目录
        </button>
        <button
          disabled={busy || !stopped || !directory.configured}
          onClick={() => setConfirm("scan")}
        >
          <Icon name="refresh" size={15} />
          手动扫描默认目录
        </button>
      </div>
      {state.directory_selection && (
        <div className="directory-pending">
          <strong>待应用目录</strong>
          <p className="directory-path">
            {state.directory_selection.display_path}
          </p>
          <p>仅保存默认下载与维护位置；保留已有模型索引，不扫描此目录中的文件。</p>
          <div className="directory-actions">
            <button
              className="primary"
              disabled={busy || !stopped}
              onClick={() => setConfirm("apply")}
            >
              设置默认下载目录
            </button>
            <button
              className="text-button"
              disabled={busy}
              onClick={controller.discardDirectorySelection}
            >
              取消选择
            </button>
          </div>
        </div>
      )}
      {!stopped && (
        <div className="directory-stop">
          <p>
            保存默认下载目录或手动扫描前，须先显式停止运行服务；仅卸载模型不够。不会自动停止其他客户端。
          </p>
          <button
            className="danger-outline"
            disabled={!running || busy}
            onClick={() => setConfirm("stop")}
          >
            先停止运行服务
          </button>
        </div>
      )}
      <div className="directory-guidance">
        <p>
          默认启动、进入模型页和刷新仅读取已登记列表，不自动扫描。手动扫描只处理默认目录直接子级中未作为显式文件来源登记的 GGUF，不递归、不扫描其他来源；单独添加的模型通过重新添加或加载复核。下载目标须可写，不会移动、重命名或删除源文件。
        </p>
        <p>
          单次最多 1024 个条目、64 个 GGUF；单文件 16 GiB、候选合计 32
          GiB、总核验时间 {state.snapshot?.configuration?.saved.runtime.model_verification_timeout_seconds ?? state.snapshot?.settings.model_verification_timeout_seconds ?? 300} 秒。超限会明确失败，原目录与索引保持不变。
        </p>
        <p className="warning-text">
          已在运行服务中使用过的外部模型，替换或重命名前须停止整个运行服务；卸载模型不会释放源文件保护。
        </p>
      </div>
      {confirm && (
        <Modal
          title={
            confirm === "stop"
              ? "停止所有客户端的运行任务？"
              : confirm === "apply"
                ? "设置默认下载目录？"
                : "手动扫描默认目录？"
          }
          confirm={
            confirm === "stop"
              ? "停止运行服务"
              : confirm === "apply"
                ? "保存默认下载目录"
                : "开始核验"
          }
          danger={confirm === "stop"}
          onCancel={() => setConfirm(null)}
          onConfirm={() => {
            const action = confirm;
            setConfirm(null);
            void (action === "stop"
              ? controller.stop()
              : action === "apply"
                ? controller.configureDirectory()
                : controller.scanModels());
          }}
        >
          {confirm === "stop" ? (
            <p>
              将停止运行服务及所有客户端任务，可能中断其他应用的调用；确认实例与 worker
              清理后才可保存默认目录或手动扫描。原文件不会移动或删除。
            </p>
          ) : (
            <p>
              {confirm === "apply"
                ? "仅保存默认下载目录，已有模型索引保留。不枚举或校验目录里的模型，不会自动启动运行服务；不会复制或删除模型文件。添加已有文件请回到模型页选择“添加模型”。"
                : "手动扫描默认目录直接子级中未作为显式文件来源登记的 GGUF，不递归、不扫描其他来源。显式添加的模型保留，通过重新添加或加载复核；本次扫描不重新校验这些文件。不支持的候选逐个报告，其余合法候选一次保存。提交前取消或全部拒绝保留旧索引。若持久化确认失败，请核对实际配置，不假定已回滚。不会自动启动运行服务，也不会复制或删除模型文件。"}
            </p>
          )}
        </Modal>
      )}
    </section>
  );
}
function ModelsPage({
  state,
  controller,
  goChat,
  goSettings,
}: {
  state: ViewState;
  controller: DesktopController;
  goChat: () => void;
  goSettings: () => void;
}) {
  const [view, setView] = useState<"local" | "download">("local");
  const [switchTo, setSwitchTo] = useState<string | null>(null);
  const projection = runtimeView(state.snapshot);
  useEffect(() => () => controller.leaveModelPage(), [controller, view]);
  const runtime = state.snapshot?.connection === "connected" ? state.snapshot.runtime : null;
  const settings = state.snapshot?.settings ?? DEFAULT_SETTINGS;
  const connected = state.snapshot?.connection === "connected";
  const browsable = connected || state.snapshot?.connection === "stopped";
  const busy =
    !!state.operation ||
    state.library_phase !== "idle" ||
    state.download_phase !== "idle" ||
    ["stale", "unsupported"].includes(
      state.snapshot?.model_directory.state ?? "",
    ) ||
    state.chat_phase !== "idle" ||
    !!runtime?.registry_busy ||
    !!runtime?.stopping ||
    !!runtime?.active_request ||
    (runtime?.queued_jobs ?? 0) > 0 ||
    ["loading", "generating", "unloading"].includes(runtime?.state ?? "");
  const loaded = projection.resident;
  const different = state.snapshot?.configuration ? !!ownRecord(state.model_configurations, projection.residentId ?? "")?.pending_apply :
    runtime?.load_options &&
    (["context_size", "threads", "batch_size"] as const).some(
      (key) => runtime.load_options![key] !== settings[key],
    );
  return (
    <>
      <div className="page-heading">
        <div>
          <span className="eyebrow">本机模型库</span>
          <h1>模型库</h1>
          <p>选择一个或多个本地 GGUF，零复制添加到模型库。</p>
        </div>
        <button className="primary" disabled={!browsable || !!state.operation || state.library_phase !== "idle" || state.download_phase !== "idle" || state.chat_phase !== "idle"} onClick={() => void controller.pickModels()}>
          <Icon name="plus" size={18} />添加模型
        </button>
      </div>
      <button className="text-button auxiliary-chat" onClick={goChat}>聊天测试</button>
      <div className="model-view-switch" role="group" aria-label="模型视图"><button aria-pressed={view === "local"} onClick={() => setView("local")}>本地模型</button><button aria-pressed={view === "download"} onClick={() => setView("download")}>下载模型</button></div>
      {view === "download" ? <ModelDownloads state={state} controller={controller} goSettings={goSettings} /> : <>
      <section className="runtime-card" aria-label="模型运行状态">
        <div className="model-emblem">
          <Icon name="models" size={26} />
        </div>
        <div className="runtime-card-main">
          <span className="overline">当前驻留</span>
          <h2>
            {projection.residentName ?? projection.modelLabel}
          </h2>
          <p>
            {projection.residentOptions
              ? `上下文 ${projection.residentOptions.context_size} · ${projection.residentOptions.threads} 线程 · 批次 ${projection.residentOptions.batch_size}`
              : "从下方选择模型，显式加载并短测后可供应用调用"}
          </p>
          {!projection.resident && projection.lastSelectedName && <p>上次选择：{projection.lastSelectedName}（不代表当前驻留）</p>}
          {loaded && different && (
            <p className="warning-text">
              当前加载参数与已保存偏好不同，新参数将在下次加载时生效。
            </p>
          )}
        </div>
        <div className="runtime-card-actions">
          <span
            className={`status-pill ${runtime?.state === "ready" ? "healthy" : ""}`}
          >
            <span className="status-dot" />
            {runtime ? stateNames[runtime.state] : "未连接"}
          </span>
          {loaded && (
            <button disabled={busy} onClick={() => void controller.unload()}>
              卸载模型
            </button>
          )}
          {runtime?.state === "ready" && (
            <button className="primary" disabled={busy} onClick={goChat}>
              验证驻留模型
              <Icon name="chevron" size={16} />
            </button>
          )}
        </div>
      </section>
      <section className="directory-summary" aria-label="模型来源目录">
        <div className="import-icon">
          <Icon name="file" size={26} />
        </div>
        <div className="import-copy">
          <h2>下载目录与维护</h2>
          <p className="directory-path">
            {state.snapshot?.model_directory.configured?.display_path ??
              "尚未设置下载目录，不影响添加已有模型"}
          </p>
          <p>刷新仅读取已登记列表。扫描默认目录须在设置中手动执行。</p>
        </div>
        <button onClick={goSettings}>
          前往目录设置
          <Icon name="chevron" size={16} />
        </button>
      </section>
      <DirectoryStateNotice state={state} />
      {state.snapshot?.configuration?.schema_version === 1 && <div className="notice-band warning"><p>旧配置尚未升级为统一档案。请先在设置确认来源，再加载模型。</p><button onClick={goSettings}>处理配置迁移</button></div>}
      {state.snapshot?.configuration?.pending_restart && <p className="warning-text">已保存配置尚未被运行实例采用，请显式停止并重启后加载。</p>}
      {state.snapshot?.connection !== "connected" &&
        state.models.data.length > 0 && (
          <p className="stale-models-note" role="status">
            {state.models.source === "local"
              ? "本地已登记模型列表。测试标签是此前的本机记录，不表示模型当前驻留内存。"
              : "以下为上次读取的列表；刷新可重新读取本地索引，加载时仍须核验文件。"}
          </p>
        )}
      {state.reconcile_status === "observing" && <p role="status" className="small-note">检测到目录变化，等待文件稳定后登记。未完成下载的文件不会作为测试通过；稍后可刷新。</p>}
      {state.reconcile_status === "pending" && <p role="status" className="warning-text">发现待登记文件。运行服务使用固定索引，请在方便时显式停止服务后刷新；不会自动中断当前模型或其他客户端。</p>}
      {state.testing_model && <p role="status" className="local-test-progress">正在加载或进行本机短文本测试。加载最多 300 秒；短文本测试最多 30 秒，取消清理可能稍后完成。测试不会写入聊天；可按已保存策略关闭应用。</p>}
      <section className="library" aria-labelledby="library-title">
        <div className="section-heading">
          <h2 id="library-title">
            可查看模型 <span className="count">{state.models.data.length}</span>
          </h2>
          <button
            className="text-button"
            disabled={!browsable || state.models_loading || busy}
            onClick={() => void controller.refreshModels()}
          >
            <Icon name="refresh" size={15} />
            刷新
          </button>
        </div>
        {state.models_loading && (
          <div className="loading-line" role="status">
            <Spinner />
            正在读取模型列表…
          </div>
        )}
        {!state.models_loading && state.models.data.length === 0 ? (
          <div className="empty-models">
            <Icon name="models" size={32} />
            <h3>你的模型库还是空的</h3>
            <p>
              点击右上方“添加模型”选择已有 GGUF，或前往“下载模型”。不会自动扫描模型目录。
            </p>
            <span>可一次选择多个完整 GGUF 文件；分片模型暂不支持</span>
          </div>
        ) : (
          <div className="model-list">
            {state.models.data.map((model) => {
              const current = model.id === runtime?.selected_model && loaded;
              const switching = loaded && !current;
              const compatibility = modelCompatibility(model);
              const attempt = ownRecord(state.model_tests, model.id);
              return (
                <article
                  key={model.id}
                  className={`model-row ${current ? "selected" : ""}`}
                >
                  <div className="model-file">
                    <Icon name="models" size={23} />
                  </div>
                  <div className="model-description">
                    <div className="model-title">
                      <h3>{model.display_name}</h3>
                      {current && <span className="mini-label">已加载</span>}
                    </div>
                    <p className="model-source">
                      {model.storage === "external"
                        ? "本地文件 · 直接读取"
                        : "原有管理模型"}
                    </p>
                    <div className="model-meta">
                      <span>{model.quantization || "量化未知"}</span>
                      <span>{model.architecture || "架构未知"}</span>
                      <span>{formatSize(model.size_bytes)}</span>
                      <span>登记时已识别 GGUF</span>
                      {state.testing_model === model.id ? <span>正在进行本机基础测试</span> : !model.local_validation && <span>{compatibility.admission}</span>}
                    </div>
                    <div className="model-id-row"><span className="hash">API ID：{model.id}</span><button aria-label={`复制 ${model.display_name} 的模型 ID`} onClick={() => void controller.copyModelId(model.id)}>复制 ID</button></div>
                    <p className="small-note">{compatibility.architecture}</p>
                    {attempt && <ModelTestFeedback attempt={attempt} />}
                    {model.local_validation ? <LocalValidationFeedback value={model.local_validation} history /> : <p className="small-note">本机记录未提供，尚未取得本机测试证明。</p>}
                    <ModelProfile modelId={model.id} state={state} controller={controller} />
                    <details>
                      <summary>模型信息</summary>

                      <p className="hash">登记时 SHA-256：{model.sha256}</p>
                      <p>历史矩阵验证记录：{model.validated ? "有精确模型验证记录" : "未实测"} · 历史验证上下文：{model.context_size ?? "未实测"}</p>
                      {model.local_validation && <>
                        <p>本机加载证明：{model.local_validation.load_success ? "曾通过" : "未取得"} · 本机短文本证明：{model.local_validation.generation_pass ? "曾通过" : "未取得"}{model.local_validation.state === "stale" ? "（记录已过期，当前组合待重测）" : ""}</p>
                      </>}
                      <p>本机基础测试仅覆盖对应文件、引擎、设备和加载参数的加载与短文本生成；不证明回答质量、长上下文、工具调用或全部功能可用。</p>
                      <p>当前上下文请求上限（模型与运行时约束）：{model.context_limit ?? "未知"}，不代表设备可承受</p>
                      <p>文件大小不等于运行内存；模型、上下文和批次越大，占用通常越高。16 GB 内存也不能保证加载成功。</p>
                      <p>工具调用和思考输出尚未开放；模型可加载不代表具备这些能力。</p>
                      <p>
                        兼容性依据登记元数据，不代表当前文件完整性；加载时仍须核验源文件。
                      </p>
                    </details>
                    {compatibility.reason && compatibility.reason !== (model.local_validation && localValidationReason(model.local_validation)) && (
                      <p className="warning-text small-note">
                        {compatibility.reason}
                        {model.availability_error &&
                          `（${model.availability_error}）`}
                      </p>
                    )}
                    {state.models.data.filter(
                      (entry) => entry.display_name === model.display_name,
                    ).length > 1 && (
                      <p className="small-note">
                        同名区分：
                        {model.storage === "external" ? "目录" : "管理"} ·{" "}
                        {model.id.slice(-8)}
                      </p>
                    )}
                    {switching && (
                      <p className="small-note">空闲时可显式切换；将释放当前模型，失败不自动恢复</p>
                    )}
                  </div>
                  <div className="model-row-actions"><button
                    className={current ? "loaded-button" : ""}
                    disabled={
                      !browsable ||
                      !!state.snapshot?.configuration_error ||
                      state.snapshot?.configuration?.schema_version === 1 ||
                      state.snapshot?.configuration?.pending_restart ||
                      busy ||
                      current ||
                      !model.available ||
                      model.loadable !== true
                    }
                    onClick={() => switching ? setSwitchTo(model.id) : void controller.loadModel(model.id)}
                  >
                    {current
                      ? "已加载"
                      : state.testing_model === model.id && attempt?.mode === "load"
                        ? "加载与测试中…"
                        : runtime?.selected_model === model.id && runtime.state === "loading"
                          ? "加载中…"
                          : !model.available ||
                      model.loadable !== true
                        ? compatibility.unavailableLabel
                        : runtime?.state === "faulted" &&
                            model.id === runtime.selected_model
                          ? "重新加载"
                          : switching ? "切换并测试" : "加载模型"}
                  </button>
                  {current && <><button disabled={busy} aria-label={`测试 ${model.display_name}`} onClick={() => void controller.testModel(model.id)}>{state.testing_model === model.id ? "测试中…" : "基础测试"}</button>{busy && state.testing_model !== model.id && <span className="small-note">当前有任务进行中，空闲后可基础测试</span>}</>}
                  {!connected && model.available && model.loadable === true && <span className="small-note">加载将启动服务并短测</span>}
                  </div>
                </article>
              );
            })}
          </div>
        )}
        <div className="pagination">
          <span>每页最多 64 个模型 · 仅保留当前页</span>
          <div>
            <button
              className="text-button"
              disabled={
                !state.page_after || state.models_loading || !browsable || busy
              }
              onClick={() => void controller.loadPage(null)}
            >
              回到首页
            </button>
            <button
              disabled={
                !state.models.next_after ||
                state.models_loading ||
                !browsable ||
                busy
              }
              onClick={() => void controller.loadPage(state.models.next_after)}
            >
              下一页
              <Icon name="chevron" size={14} />
            </button>
          </div>
        </div>
      </section>
      {switchTo && <Modal title="切换驻留模型？" confirm="切换并测试" onCancel={() => setSwitchTo(null)} onConfirm={() => { const id = switchTo; setSwitchTo(null); void controller.loadModel(id); }}><p>将释放当前模型，加载所选模型并进行短文本测试。忙碌时服务会拒绝切换；加载失败不会自动恢复旧模型，请查看真实状态后显式重载。</p></Modal>}
      <div className="footnote">
        <Icon name="shield" size={16} />
        <p>
          配置后端：CPU · 原生后端观测：{runtime?.backend ?? "unavailable"} ·
          内存观测：{runtime?.memory?.observation ?? "unavailable"}
          <br />
          {state.snapshot?.configuration ? "加载参数由后端按已保存的全局默认与模型档案解析，不代表硬件性能保证。" : "旧版本加载参数来自桌面偏好；逐模型统一档案需要更新应用。"}
        </p>
      </div>
      </>}
    </>
  );
}
function ChatPage({
  state,
  controller,
  draft,
  setDraft,
  goModels,
}: {
  state: ViewState;
  controller: DesktopController;
  draft: string;
  setDraft: (value: string) => void;
  goModels: () => void;
}) {
  const projection = runtimeView(state.snapshot);
  const requestDefaults = state.snapshot?.configuration?.runtime_effective?.values.request_defaults;
  const outputBudget = requestDefaults?.max_output_tokens ?? state.snapshot?.settings.max_output_tokens;
  const composing = useRef(false);
  const tail = useRef<HTMLDivElement>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const autoScroll = useRef(true);
  const active = state.chat_phase !== "idle";
  const ready =
    state.snapshot?.connection === "connected" &&
    state.snapshot.runtime?.state === "ready";
  const incomplete = state.messages.some(
    (message) => message.state === "incomplete",
  );
  const canSend =
    ready &&
    !active &&
    !state.operation &&
    state.library_phase === "idle" &&
    state.download_phase === "idle" &&
    !["stale", "unsupported"].includes(
      state.snapshot?.model_directory.state ?? "",
    ) &&
    !!draft.trim() &&
    !incomplete;
  const [confirmClear, setConfirmClear] = useState(false);
  const latest = state.messages.at(-1)?.content;
  useEffect(() => {
    if (autoScroll.current) tail.current?.scrollIntoView({ block: "end" });
  }, [latest, state.messages.length]);
  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    if (!canSend || composing.current) return;
    const accepted = await controller.send(draft);
    if (accepted) setDraft("");
  };
  return (
    <div className="chat-page">
      <div className="page-heading chat-heading">
        <div>
          <span className="eyebrow">本地推理会话</span>
          <h1>聊天测试</h1>
          <p>
            {projection.residentName ?? (projection.lastSelectedName ? `上次选择：${projection.lastSelectedName}（当前未确认驻留）` : projection.modelLabel)}
            <span className="inline-dot">·</span>会话仅保留在当前窗口
          </p>
          <p aria-label="辅助测试实际参数">实际上下文：{projection.residentOptions?.context_size ?? "未报告驻留参数"} · 本次输出预算：{outputBudget ?? "未知"} tokens（服务默认）</p>
          <p className="small-note">temperature：{requestDefaults?.temperature ?? "未报告"} · top_p：{requestDefaults?.top_p ?? "未报告"}（使用服务请求默认值，不覆盖已受理的请求）</p>
        </div>
        <button
          disabled={!state.messages.length || state.clear_pending}
          onClick={() => setConfirmClear(true)}
        >
          {state.clear_pending ? <Spinner /> : <Icon name="close" size={16} />}
          {state.clear_pending ? "停止后清空…" : "清空会话"}
        </button>
      </div>
      <div
        className="transcript"
        role="region"
        aria-label="当前会话"
        ref={scroll}
        onScroll={() => {
          const el = scroll.current;
          if (el)
            autoScroll.current =
              el.scrollHeight - el.scrollTop - el.clientHeight < 80;
        }}
      >
        {!state.messages.length ? (
          <div className="chat-empty">
            <Brand />
            <h2>从一句话开始</h2>
            <p>消息由本机模型处理。关闭窗口后，本次会话不保留。</p>
            {ready ? (
              <div className="prompt-suggestions">
                {[
                  "用简单的话解释什么是本地推理",
                  "帮我写一段简短的项目介绍",
                ].map((prompt) => (
                  <button key={prompt} onClick={() => setDraft(prompt)}>
                    {prompt}
                    <Icon name="chevron" size={15} />
                  </button>
                ))}
              </div>
            ) : (
              <button onClick={goModels} className="primary">
                <Icon name="models" size={17} />
                先去加载模型
              </button>
            )}
          </div>
        ) : (
          <div className="messages">
            {state.messages.map((message) => (
              <article
                className={`message ${message.role}`}
                key={message.id}
                aria-label={message.role === "user" ? "你的消息" : "Nexa 回复"}
              >
                <div className="message-avatar">
                  {message.role === "user" ? "你" : <Brand small />}
                </div>
                <div className="message-main">
                  <div className="message-name">
                    {message.role === "user" ? "你" : "Nexa"}
                    {message.state === "incomplete" && (
                      <span className="incomplete-label">不完整</span>
                    )}
                  </div>
                  <div className="message-text">
                    {message.content ||
                      (message.state === "streaming" ? (
                        <span className="pending-text">
                          <Spinner />
                          {state.chat_phase === "starting"
                            ? "正在提交请求…"
                            : state.chat_phase === "stopping"
                              ? "正在等待停止确认…"
                              : "正在等待模型输出…"}
                        </span>
                      ) : (
                        "（没有返回文本）"
                      ))}
                  </div>
                  {message.notice && (
                    <p
                      className={`message-note ${message.state === "incomplete" ? "warning-text" : ""}`}
                    >
                      {message.notice}
                    </p>
                  )}
                  {message.usage && (
                    <p className="message-note">
                      输入 {message.usage.prompt_tokens} · 输出{" "}
                      {message.usage.completion_tokens} tokens
                      {message.finish_reason === "length"
                        ? " · 达到输出预算"
                        : ""}
                    </p>
                  )}
                </div>
              </article>
            ))}
            <div ref={tail} />
          </div>
        )}
      </div>
      {state.chat_phase === "recovery" && (
        <div className="chat-alert" role="status">
          <div>
            <strong>连接中断，生成终态尚未确认</strong>
            <p>已有文本已标为不完整。确认停止前不会清空，也不会重新发送。</p>
          </div>
          <button onClick={() => void controller.recover()}>
            重新确认停止
          </button>
        </div>
      )}
      {incomplete && !active && (
        <div className="chat-alert">
          <span>请清空会话或移除未完成轮次后再发送。</span>
          <button onClick={controller.removeIncomplete}>移除未完成轮次</button>
        </div>
      )}
      <form className="composer" onSubmit={(event) => void submit(event)}>
        <label htmlFor="chat-input" className="sr-only">
          输入消息
        </label>
        <textarea
          id="chat-input"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          maxLength={LIMITS.session}
          placeholder={
            ready ? "输入消息，开始本地对话…" : "加载模型后即可发送消息…"
          }
          rows={3}
          disabled={active || !!state.operation}
          onCompositionStart={() => {
            composing.current = true;
          }}
          onCompositionEnd={() => {
            composing.current = false;
          }}
          onKeyDown={(event) => {
            if (
              event.key === "Enter" &&
              !event.shiftKey &&
              !event.nativeEvent.isComposing &&
              event.keyCode !== 229 &&
              !composing.current
            ) {
              event.preventDefault();
              void submit();
            }
          }}
        />
        <div className="composer-bottom">
          <span>Enter 发送 · Shift + Enter 换行</span>
          <div className="composer-actions">
            {active && (
              <button
                type="button"
                className="stop-button"
                disabled={
                  state.chat_phase === "stopping" ||
                  state.chat_phase === "recovery"
                }
                onClick={() => void controller.cancel()}
              >
                {state.chat_phase === "stopping" ? (
                  <Spinner />
                ) : (
                  <Icon name="stop" size={15} />
                )}
                {state.chat_phase === "stopping" ? "正在停止…" : "停止生成"}
              </button>
            )}
            <button
              type="submit"
              className="primary send-button"
              disabled={!canSend}
            >
              发送
              <Icon name="arrow" size={17} />
            </button>
          </div>
        </div>
      </form>
      <div className="chat-footer">
        <span>模型输出可能不准确，请核对重要信息</span>
        <span>
          {state.messages.length} / 128 条 ·{" "}
          {(
            state.messages.reduce(
              (sum, message) => sum + byteLength(message.content),
              0,
            ) / 1024
          ).toFixed(1)}{" "}
          / 512 KiB
        </span>
      </div>
      {confirmClear && (
        <Modal
          title="清空当前会话？"
          confirm={active ? "停止并清空" : "清空会话"}
          danger
          onCancel={() => setConfirmClear(false)}
          onConfirm={() => {
            setConfirmClear(false);
            void controller.clear();
          }}
        >
          <p>
            {active
              ? "将先取消本窗口的生成，收到终态后再清空。其他客户端不受影响。"
              : "本次会话的消息将被移除，无法恢复。已加载模型保持不变。"}
          </p>
        </Modal>
      )}
    </div>
  );
}
function SettingsPage({
  settings,
  state,
  controller,
}: {
  settings: Settings;
  state: ViewState;
  controller: DesktopController;
}) {
  const form = useConfigDraft(preferencesOnly(settings), undefined, "legacy_preferences");
  const { draft, setDraft } = form;
  const runtime = state.snapshot?.runtime;
  const validation = validatePreferences(draft);
  const fields: {
    key: "context_size" | "threads" | "batch_size" | "max_output_tokens";
    label: string;
    detail: string;
    min: number;
    max: number;
  }[] = [
    {
      key: "context_size",
      label: "上下文长度",
      detail: "输入、模板与输出共用 token 预算；上限不是内存保证，大上下文可能加载失败",
      min: 32,
      max: 131072,
    },
    {
      key: "threads",
      label: "推理线程",
      detail: "按设备调整，更多线程不保证更快",
      min: 1,
      max: 256,
    },
    {
      key: "batch_size",
      label: "批次大小",
      detail: "每批处理的 token 数量",
      min: 1,
      max: Math.min(draft.context_size, 4096),
    },
    {
      key: "max_output_tokens",
      label: "默认输出预算",
      detail: "每次发送的最大输出 tokens",
      min: 1,
      max: 4096,
    },
  ];
  return (
    <>
      <div className="page-heading">
        <div>
          <span className="eyebrow">运行与偏好</span>
          <h1>设置</h1>
          <p>未保存草稿切页保留，并尽力保存非秘密恢复副本；重开须明确恢复。保存后再按字段说明显式应用。</p>
        </div>
      </div>
      <DirectorySettings state={state} controller={controller} />
      {form.conflict && <div className="notice-band warning" role="alert"><div><strong>已保存配置发生变化</strong><p>保留了你的未保存草稿。请先重新读取新配置，再决定如何修改，避免覆盖其他窗口的更新。</p></div><button onClick={form.reset}>丢弃草稿并读取新配置</button></div>}
      {!state.snapshot?.initialized && state.snapshot?.connection === "stopped" && <section className="settings-card"><h2>先准备配置</h2><p>初始化本机配置与管理凭据，不启动服务、监听端口或扫描模型。</p><button disabled={!!state.operation} onClick={() => void controller.initialize()}>仅初始化配置</button></section>}
      {state.snapshot?.configuration && <><GlobalConfiguration state={state} controller={controller} /><RequestDefaultsForm state={state} controller={controller} /></>}
      {state.snapshot?.ui_preferences && <UiPreferencesForm state={state} controller={controller} />}
      {!state.snapshot?.configuration && !state.snapshot?.configuration_error && <form
        onSubmit={(event) => {
          event.preventDefault();
          if (!form.conflict) void controller.saveSettings(draft);
        }}
      >
        <section className="settings-card">
          <label className="setting-field" htmlFor="download-source"><span>默认下载源</span><small>默认 ModelScope，可改为 Hugging Face。点击下方“保存偏好”后生效；在途下载不会切换来源。</small>
            <select id="download-source" value={draft.download_source} onChange={(event) => setDraft({ ...draft, download_source: event.target.value as Preferences["download_source"] })}>
              <option value="modelscope">ModelScope</option><option value="huggingface">Hugging Face</option>
            </select>
          </label>
        </section>
        <section className="settings-card">
          <div className="card-heading">
            <div>
              <h2>推理偏好</h2>
              <p>加载参数在下次加载时生效，输出预算用于下次发送。</p>
            </div>
            <span className="subtle-pill">CPU</span>
          </div>
          <div className="settings-grid">
            {fields.map(({ key, label, detail, min, max }) => (
              <label className="setting-field" key={key} htmlFor={key}>
                <span>{label}</span>
                <small>{detail}</small>
                <div className="number-field">
                  <input
                    type="number"
                    id={key}
                    min={min}
                    max={max}
                    step={1}
                    value={Number.isNaN(draft[key]) ? "" : draft[key]}
                    onChange={(event) =>
                      setDraft({ ...draft, [key]: event.target.valueAsNumber })
                    }
                  />
                  <span>{key === "threads" ? "线程" : "tokens"}</span>
                </div>
              </label>
            ))}
          </div>
          <div className="current-config">
            <Icon name="models" size={17} />
            <p>
              {runtime && ["ready", "generating"].includes(runtime.state) && runtime.load_options
                ? `当前已加载：上下文 ${runtime.load_options.context_size} / ${runtime.load_options.threads} 线程 / 批次 ${runtime.load_options.batch_size}`
                : "当前没有已加载参数"}
              <br />
              <span>
                固定 CPU 后端，GPU layers = 0；实际原生后端观测：
                {runtime?.backend ?? "unavailable"}
              </span>
            </p>
          </div>
        </section>
        <section className="settings-card">
          <div className="toggle-row">
            <div>
              <h2>关闭窗口时同时退出运行服务</h2>
              <p>
                默认关闭窗口仅取消本窗口生成，运行服务继续供其他客户端使用。
              </p>
            </div>
            <input
              className="switch"
              type="checkbox"
              role="switch"
              aria-label="关闭窗口时同时退出运行服务"
              checked={draft.close_runtime_on_exit}
              onChange={(event) =>
                setDraft({
                  ...draft,
                  close_runtime_on_exit: event.target.checked,
                })
              }
            />
          </div>
          {draft.close_runtime_on_exit && (
            <p className="warning-text">
              启用并保存后，关闭窗口会停止运行服务及所有客户端任务。
            </p>
          )}
        </section>
        <div className="save-row">
          <span className={validation ? "warning-text" : "muted"}>
            {validation ?? "偏好保存不会改变当前已加载模型。"}
          </span>
          <button
            type="submit"
            className="primary"
            disabled={
              !!state.operation ||
              state.library_phase !== "idle" ||
              state.download_phase !== "idle" ||
              !!validation || form.conflict
            }
          >
            {state.operation?.label === "正在保存偏好" && <Spinner />}保存偏好
          </button>
        </div>
      </form>}
      <IdleUnloadSettings state={state} controller={controller} />
      <VerificationTimeoutSettings state={state} controller={controller} />

    </>
  );
}
export default function App({
  controller,
  preview = false,
  initialPage = "overview",
}: {
  controller: DesktopController;
  preview?: boolean;
  initialPage?: "overview" | "models" | "api" | "activity" | "chat" | "settings";
}) {
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const [page, setPage] = useState(initialPage);
  const [configDrafts] = useState(createDraftStore);
  const [confirmClose, setConfirmClose] = useState(false);
  const [draftEpoch, setDraftEpoch] = useState(0);
  useSyncExternalStore(configDrafts.subscribe, configDrafts.getSnapshot);
  const [draft, setDraft] = useState("");
  const [confirmAddStop, setConfirmAddStop] = useState(false);
  const [confirmStop, setConfirmStop] = useState(false);
  const [confirmRecoveryStop, setConfirmRecoveryStop] = useState(false);
  useEffect(() => controller.mount(), [controller]);
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.altKey && ["1", "2", "3", "4", "5"].includes(event.key)) {
        event.preventDefault();
        setConfirmStop(false);
        setPage(
          (["overview", "models", "api", "activity", "settings"] as const)[Number(event.key) - 1],
        );
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, []);
  const nav = [
    { id: "overview", title: "概览", detail: "状态与下一步", icon: "shield" },
    { id: "models", title: "模型库", detail: "文件与运行档案", icon: "models" },
    { id: "api", title: "API 接入", detail: "连接你的应用", icon: "copy" },
    { id: "activity", title: "活动", detail: "任务与结果", icon: "refresh" },
  ] as const;
  const healthy =
    state.snapshot?.connection === "connected" &&
    state.snapshot.runtime?.state !== "faulted";
  return (
    <ConfigDraftContext.Provider value={configDrafts}><div className="app-shell">
      <aside className="sidebar">
        <div className="sidebar-navigation">
        <div className="brand">
          <Brand />
          <div>
            <strong>Nexa</strong>
            <span>你的本地推理空间</span>
          </div>
        </div>
        <div className="nav-label">工作空间</div>
        <nav aria-label="主导航">
          {nav.map((item, index) => (
            <button
              key={item.id}
              aria-current={page === item.id ? "page" : undefined}
              aria-label={item.title}
              title={`${item.title}（Alt + ${index + 1}）`}
              className={`nav-item ${page === item.id ? "active" : ""}`}
              onClick={() => { setConfirmStop(false); setPage(item.id); }}
            >
              <Icon name={item.icon} />
              <span>
                <strong>{item.title}</strong>
                <small>{item.detail}</small>
              </span>
              {item.id === "activity" && (state.chat_phase !== "idle" || !!state.testing_model) && (
                <span className="nav-live" />
              )}
            </button>
          ))}
        </nav>
        </div>
        <div className="sidebar-bottom">
          <button className={`nav-item ${page === "settings" ? "active" : ""}`} aria-label="设置" aria-current={page === "settings" ? "page" : undefined} title="设置（Alt + 5）" onClick={() => setPage("settings")}><Icon name="settings" /><span><strong>设置</strong><small>全局默认与偏好</small></span></button>
          <div className="local-card">
            <Icon name="shield" size={18} />
            <strong>留在你的设备上</strong>
            <p>
              模型在本机运行
              <br />
              聊天不会保存到磁盘
            </p>
          </div>
          <ServiceControl state={state} controller={controller} onStop={() => setConfirmStop(true)} />
          <div className="sidebar-footer">
            <span>Windows · CPU</span>
            <button
              title={state.snapshot?.settings.close_runtime_on_exit ? "停止服务并退出窗口" : "关闭窗口并保留服务"}
              aria-label={state.snapshot?.settings.close_runtime_on_exit ? "停止服务并退出窗口" : "关闭窗口并保留服务"}
              className="icon-button"
              disabled={!!state.operation && !state.testing_model && state.operation.kind !== "pick_models"}
              onClick={() => { if (hasDirtyDrafts(configDrafts)) setConfirmClose(true); else void controller.close(); }}
            >
              <Icon name="power" size={16} />
            </button>
          </div>
        </div>
      </aside>
      <div className="workspace">
        <header className="topbar">
          <span>
            Nexa <span className="breadcrumb-slash">/</span>{" "}
            {nav.find((item) => item.id === page)?.title ?? (page === "settings" ? "设置" : "聊天测试")}
          </span>
          <div
            className={`connection ${healthy ? "connected" : ""}`}
            role="status"
          >
            {state.operation || state.booting ? (
              <Spinner />
            ) : (
              <span className="status-dot" />
            )}
            {statusLabel(state)}
          </div>
        </header>
        {preview && (
          <div className="preview-banner">
            开发预览 · 全部运行数据与回复为模拟，仅用于 UI 检查，不代表真实推理
          </div>
        )}
        <main key={draftEpoch}
          className={
            page === "chat" ? "main-content chat-content" : "main-content"
          }
        >
          <RuntimeBanner state={state} />
          {state.activity_storage_warning && <div className="notice-band warning" role="alert"><p>{state.activity_storage_warning}</p></div>}
          {state.configuration_recovery_error && <section className="notice-band warning" role="alert"><div><strong>配置无法读取，服务状态尚未确认</strong><p>不会用默认值代替配置或启动新实例。可检查并停止当前本机实例；原生端仍会验证身份并确认清理，其他客户端可能受影响。</p><p>配置诊断码：{state.configuration_recovery_error.code}</p><button disabled={!!state.operation || state.chat_phase !== "idle" || state.library_phase !== "idle" || state.download_phase !== "idle"} onClick={() => setConfirmRecoveryStop(true)}>检查并停止本机服务</button></div></section>}
          {configDrafts.pending.size > 0 && <section className="notice-band warning" role="status"><div><strong>发现上次未保存草稿</strong><p>可恢复 {configDrafts.pending.size} 组非秘密配置草稿。请先选择恢复或丢弃，再编辑配置。它们不是已保存配置；恢复后会核对原版本，不会自动提交、加载或启动服务。</p><div className="workspace-actions"><button onClick={() => { configDrafts.restorePending(); setDraftEpoch((value) => value + 1); }}>恢复未保存草稿</button><button onClick={() => configDrafts.discardPending()}>丢弃上次草稿</button></div></div></section>}
          {configDrafts.warning && <div className="notice-band warning" role="alert"><p>{configDrafts.warning}</p></div>}
          {state.snapshot?.configuration_error && <div className="notice-band warning" role="alert"><div><strong>统一配置暂不可用</strong><p>当前服务连接与驻留状态仍保留；请检查后台版本，必要时显式停止服务后重新启动匹配版本。不会自动替换实例或创建空档案。</p><p>诊断码：{state.snapshot.configuration_error.code}</p></div><button disabled={!!state.operation} onClick={() => void controller.checkService()}>重新核对配置能力</button></div>}
          {(["stale", "unsupported"].includes(
            state.snapshot?.model_directory.state ?? "",
          ) ||
            state.error?.code === "model_library_unsupported") && (
            <div className="notice-band warning">
              <div>
                <strong>当前模型库尚不可使用</strong>
                <p>
                  已保存目录与当前服务不匹配，或当前服务不支持模型库版本。请先显式停止，再启动匹配版本；不会自动重发聊天。
                </p>
              </div>
              <button onClick={() => setPage("settings")}>查看运行设置</button>
            </div>
          )}
          {state.error && (
            <div className="error-banner" role="alert">
              <div>
                <strong>操作未完成</strong>
                <p>{state.error.message}</p>
                {state.library?.status === "failed" &&
                  state.library.error?.code === state.error.code &&
                  state.library.failed_file_name && (
                    <p className="failed-file-name">
                      失败文件：
                      <strong>{state.library.failed_file_name}</strong>
                    </p>
                  )}
                <span>{state.error.code}</span>
              </div>
              <button
                className="icon-button"
                aria-label="收起错误"
                onClick={controller.dismissError}
              >
                <Icon name="close" size={18} />
              </button>
            </div>
          )}
          {state.library?.status === "failed" &&
            state.library.error?.code === "settings_durability_unconfirmed" && (
              <div className="notice-band warning" role="alert">
                <div>
                  <strong>{state.library_kind === "add" ? "新增索引持久化尚未确认" : "模型目录持久化尚未确认"}</strong>
                  <p>
                    {state.library_kind === "add" ? "新增登记可能已经发布，请刷新核对实际模型列表；这不代表已回滚。不会自动重新添加。" : "目录索引可能已经替换，不能保证旧目录仍在，也不代表已回滚。请核对设置中的已保存目录；若状态读取失败，请先重新检查。不会自动重新应用目录。"}
                  </p>
                </div>
                <button
                  disabled={!!state.operation}
                  onClick={() => void controller.refresh()}
                >
                  重新读取配置
                </button>
              </div>
            )}
          <DownloadProgress state={state} controller={controller} />
          <ModelSelectionPanel state={state} controller={controller} onStop={() => setConfirmAddStop(true)} />
          <AddModelProgress state={state} controller={controller} />
          <LibraryProgress state={state} controller={controller} />
          <LibraryDiagnostics state={state} />
          {state.notice && (
            <div className="success-notice" role="status">
              {state.notice}
            </div>
          )}
          <fieldset className="workspace-pages" disabled={configDrafts.pending.size > 0}>
          {page === "overview" && <OverviewPage state={state} controller={controller} goModels={() => setPage("models")} goApi={() => setPage("api")} goActivity={() => setPage("activity")} goChat={() => setPage("chat")} />}
          {page === "api" && <ApiPage state={state} controller={controller} goChat={() => setPage("chat")} />}
          {page === "activity" && <ActivityPage state={state} controller={controller} goChat={() => setPage("chat")} />}
          {page === "models" && (
            <ModelsPage
              state={state}
              controller={controller}
              goChat={() => setPage("chat")}
              goSettings={() => setPage("settings")}
            />
          )}
          {page === "chat" && (
            <ChatPage
              state={state}
              controller={controller}
              draft={draft}
              setDraft={setDraft}
              goModels={() => setPage("models")}
            />
          )}
          {page === "settings" && state.snapshot && (
            <SettingsPage
              settings={state.snapshot.settings}
              state={state}
              controller={controller}
            />
          )}
          </fieldset>
          {confirmRecoveryStop && <Modal title="检查并停止本机服务？" confirm="确认检查并停止" danger onCancel={() => setConfirmRecoveryStop(false)} onConfirm={() => { setConfirmRecoveryStop(false); void controller.recoverStopService(); }}><p>此操作可能终止其他客户端的请求并释放驻留模型。仅尝试经过原生身份验证的停止与清理，不会启动服务、重置凭据或修复损坏配置。状态与清理未确认时不会宣称已停止。</p></Modal>}
          {confirmClose && <Modal title="放弃未保存草稿并关闭窗口？" confirm="放弃草稿并关闭" danger onCancel={() => setConfirmClose(false)} onConfirm={() => { if (configDrafts.discardAll()) { setConfirmClose(false); void controller.close(); } }}><p>本窗口有未保存的配置草稿。确认后会清除恢复草稿并关闭，不会自动保存到后端。{state.snapshot?.settings.close_runtime_on_exit ? "已保存策略会停止运行服务及所有客户端任务。" : "已保存策略会保留运行服务，并取消本窗口工作。"}</p></Modal>}
          {confirmStop && <Modal title="停止所有客户端的运行任务？" confirm="停止运行服务" danger onCancel={() => setConfirmStop(false)}
            onConfirm={() => { setConfirmStop(false); void controller.stop(); }}>
            <p>这会停止本机运行服务，终止所有客户端的任务并卸载模型，可能中断其他应用正在进行的调用。收到清理确认后才会显示已停止。</p><p>当前执行请求：{state.snapshot?.runtime?.active_request ? "1 项" : state.snapshot?.runtime ? "0 项" : "未知"}；排队请求：{state.snapshot?.runtime?.queued_jobs ?? "未知"} 项。停止前状态仍可能变化。</p>
          </Modal>}
          {confirmAddStop && <Modal title="停止所有客户端的运行任务？" confirm="停止运行服务" danger onCancel={() => setConfirmAddStop(false)} onConfirm={() => { setConfirmAddStop(false); void controller.stop(); }}><p>将终止所有客户端任务并卸载当前模型，可能中断其他应用的调用。确认停止后，所选文件保留，请再次点击添加；不会自动开始登记。</p></Modal>}
        </main>
      </div>
    </div></ConfigDraftContext.Provider>
  );
}
