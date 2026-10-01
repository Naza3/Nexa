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
  if (state.booting) return "正在连接";
  if (state.operation) return state.operation;
  if (state.chat_phase === "stopping") return "正在停止生成";
  if (state.chat_phase === "recovery") return "连接中断 · 待确认";
  const snapshot = state.snapshot;
  if (!snapshot) return "桌面连接不可用";
  if (!snapshot.initialized) return "尚未初始化";
  if (snapshot.connection === "error") return "连接失效";
  if (snapshot.connection === "stopped") return "运行服务已停止";
  if (snapshot.connection === "connecting") return "正在启动";
  return snapshot.runtime ? stateNames[snapshot.runtime.state] : "等待运行状态";
}
function formatSize(bytes: number) {
  return bytes >= 1024 ** 3
    ? `${(bytes / 1024 ** 3).toFixed(2)} GiB`
    : `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
}
function Modal({
  title,
  children,
  confirm,
  onConfirm,
  onCancel,
  danger = false,
}: {
  title: string;
  children: ReactNode;
  confirm: string;
  onConfirm: () => void;
  onCancel: () => void;
  danger?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const handle = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancel();
      }
      if (event.key === "Tab") {
        const items = ref.current?.querySelectorAll<HTMLButtonElement>(
          "button:not(:disabled)",
        );
        if (!items?.length) return;
        const first = items[0],
          last = items[items.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener("keydown", handle);
    return () => {
      document.removeEventListener("keydown", handle);
      previous?.focus();
    };
  }, [onCancel]);
  return (
    <div className="modal-backdrop">
      <div
        ref={ref}
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="dialog-title"
      >
        <span className="eyebrow">请确认</span>
        <h2 id="dialog-title">{title}</h2>
        <div className="modal-copy">{children}</div>
        <div className="modal-actions">
          <button onClick={onCancel}>取消</button>
          <button
            className={danger ? "danger-button" : "primary"}
            onClick={onConfirm}
          >
            {confirm}
          </button>
        </div>
      </div>
    </div>
  );
}
function RuntimeBanner({
  state,
  controller,
}: {
  state: ViewState;
  controller: DesktopController;
}) {
  const snapshot = state.snapshot;
  if (state.booting)
    return (
      <div className="notice-band">
        <Spinner />
        正在检查本机运行服务…
      </div>
    );
  if (!snapshot)
    return (
      <div className="notice-band warning">
        <div>
          <strong>尚未连接到桌面服务</strong>
          <p>请在 Nexa 桌面应用中打开，或重新检查连接。</p>
        </div>
        <button onClick={() => void controller.refresh()}>
          <Icon name="refresh" size={16} />
          重新检查
        </button>
      </div>
    );
  if (!snapshot.initialized)
    return (
      <div className="notice-band">
        <div>
          <strong>第一次使用 Nexa</strong>
          <p>初始化本机数据目录，再启动运行服务。已有凭据不会被轮换。</p>
        </div>
        <button
          className="primary"
          disabled={!!state.operation}
          onClick={() => void controller.start(true)}
        >
          {state.operation ? <Spinner /> : <Icon name="power" size={16} />}
          初始化并启动
        </button>
      </div>
    );
  if (snapshot.connection !== "connected")
    return (
      <div
        className={`notice-band ${snapshot.connection === "error" ? "warning" : ""}`}
      >
        <div>
          <strong>
            {snapshot.connection === "error"
              ? "与运行服务的连接失效"
              : "运行服务尚未就绪"}
          </strong>
          <p>
            {snapshot.connection === "error"
              ? "先重新检查连接。无法验证的现有实例不会被替换，请检查后重试。"
              : "启动本机服务后，即可管理模型和进行聊天。"}
          </p>
        </div>
        <button
          className="primary"
          disabled={!!state.operation}
          onClick={() =>
            void (snapshot.connection === "error"
              ? controller.refresh()
              : controller.start(false))
          }
        >
          {state.operation ? <Spinner /> : <Icon name="power" size={16} />}
          {snapshot.connection === "error" ? "重新检查连接" : "启动运行服务"}
        </button>
      </div>
    );
  if (snapshot.runtime?.state === "faulted")
    return (
      <div className="notice-band warning">
        <div>
          <strong>模型运行故障</strong>
          <p>
            {snapshot.runtime.last_error?.message ??
              "请在模型页显式重新加载，恢复后再发送。已有请求不会重放。"}
          </p>
        </div>
      </div>
    );
  return null;
}
function ModelsPage({
  state,
  controller,
  goChat,
}: {
  state: ViewState;
  controller: DesktopController;
  goChat: () => void;
}) {
  const [modelId, setModelId] = useState("");
  const runtime = state.snapshot?.runtime;
  const settings = state.snapshot?.settings ?? DEFAULT_SETTINGS;
  const connected = state.snapshot?.connection === "connected";
  const busy =
    !!state.operation ||
    state.chat_phase !== "idle" ||
    !!runtime?.registry_busy ||
    !!runtime?.stopping ||
    ["loading", "generating", "unloading"].includes(runtime?.state ?? "");
  const loaded = runtime?.state === "ready" || runtime?.state === "generating";
  const different =
    runtime?.load_options &&
    (["context_size", "threads", "batch_size"] as const).some(
      (key) => runtime.load_options![key] !== settings[key],
    );
  return (
    <>
      <div className="page-heading">
        <div>
          <span className="eyebrow">本机模型库</span>
          <h1>模型</h1>
          <p>导入你的 GGUF 模型，在这台电脑上运行。</p>
        </div>
        <span className="subtle-pill">
          <Icon name="shield" size={15} />
          本地运行
        </span>
      </div>
      <section className="runtime-card" aria-label="模型运行状态">
        <div className="model-emblem">
          <Icon name="models" size={26} />
        </div>
        <div className="runtime-card-main">
          <span className="overline">当前加载</span>
          <h2>{runtime?.selected_model ?? "尚未加载模型"}</h2>
          <p>
            {runtime?.load_options
              ? `上下文 ${runtime.load_options.context_size} · ${runtime.load_options.threads} 线程 · 批次 ${runtime.load_options.batch_size}`
              : "从下方选择模型，加载后即可聊天"}
          </p>
          {different && (
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
            <button className="primary" onClick={goChat}>
              开始聊天
              <Icon name="chevron" size={16} />
            </button>
          )}
        </div>
      </section>
      <section className="import-card" aria-labelledby="import-title">
        <div className="import-icon">
          <Icon name="file" size={26} />
        </div>
        <div className="import-copy">
          <h2 id="import-title">导入本地模型</h2>
          <p>选择 .gguf 文件，Nexa 会复制到本机模型管理目录。</p>
        </div>
        <button
          disabled={!connected || busy}
          onClick={() => void controller.pick()}
        >
          <Icon name="plus" size={17} />
          选择 GGUF 文件
        </button>
        {state.selection && (
          <form
            className="import-form"
            onSubmit={(event) => {
              event.preventDefault();
              void controller.importModel(modelId);
            }}
          >
            <div className="selected-file">
              <Icon name="file" size={19} />
              <strong>{state.selection.file_name}</strong>
              <span>{formatSize(state.selection.size_bytes)}</span>
            </div>
            <p className="path-line">
              复制到：<span>{state.selection.destination}</span>
            </p>
            <label htmlFor="model-id">
              模型 ID{" "}
              <span className="label-hint">小写字母、数字、.、_ 或 -</span>
            </label>
            <div className="input-action">
              <input
                id="model-id"
                value={modelId}
                maxLength={64}
                placeholder="例如 qwen3-0.6b-q8"
                disabled={busy}
                onChange={(event) => setModelId(event.target.value)}
              />
              <button
                type="submit"
                className="primary"
                disabled={busy || !/^[a-z0-9][a-z0-9._-]{0,63}$/.test(modelId)}
              >
                {state.operation === "正在导入模型" ? (
                  <Spinner />
                ) : (
                  <Icon name="plus" size={16} />
                )}
                {state.operation === "正在导入模型" ? "正在导入…" : "确认导入"}
              </button>
              <button
                type="button"
                className="text-button"
                disabled={busy}
                onClick={controller.discardSelection}
              >
                取消
              </button>
            </div>
            {state.operation === "正在导入模型" && (
              <p role="status">
                正在复制并校验文件，请稍候。服务未提供字节进度。
              </p>
            )}
          </form>
        )}
      </section>
      <section className="library" aria-labelledby="library-title">
        <div className="section-heading">
          <h2 id="library-title">
            已导入模型 <span className="count">{state.models.data.length}</span>
          </h2>
          <button
            className="text-button"
            disabled={!connected || state.models_loading || busy}
            onClick={() => void controller.loadPage(state.page_after)}
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
            <p>选择上方的 GGUF 文件，添加第一个模型。</p>
            <span>模型文件由你提供，不会自动下载</span>
          </div>
        ) : (
          <div className="model-list">
            {state.models.data.map((model) => {
              const current = model.id === runtime?.selected_model && loaded;
              const mustUnload = loaded && !current;
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
                    <p>{model.id}</p>
                    <div className="model-meta">
                      <span>{model.quantization || "量化未知"}</span>
                      <span>{model.architecture || "架构未知"}</span>
                      <span>{formatSize(model.size_bytes)}</span>
                      <span>{model.validated ? "已校验" : "尚未校验"}</span>
                    </div>
                    <details>
                      <summary>模型信息</summary>
                      <p className="hash">SHA-256：{model.sha256}</p>
                      <p>模型上下文：{model.context_size ?? "未知"}</p>
                    </details>
                    {mustUnload && (
                      <p className="small-note">加载前请先卸载当前模型</p>
                    )}
                  </div>
                  <button
                    className={current ? "loaded-button" : ""}
                    disabled={
                      !connected ||
                      busy ||
                      current ||
                      mustUnload ||
                      !model.available
                    }
                    onClick={() => void controller.loadModel(model.id)}
                  >
                    {current
                      ? "已加载"
                      : !model.available
                        ? "当前不可用"
                        : runtime?.state === "faulted" &&
                            model.id === runtime.selected_model
                          ? "重新加载"
                          : "加载模型"}
                  </button>
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
                !state.page_after || state.models_loading || !connected || busy
              }
              onClick={() => void controller.loadPage(null)}
            >
              回到首页
            </button>
            <button
              disabled={
                !state.models.next_after ||
                state.models_loading ||
                !connected ||
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
      <div className="footnote">
        <Icon name="shield" size={16} />
        <p>
          配置后端：CPU · 原生后端观测：{runtime?.backend ?? "unavailable"} ·
          内存观测：{runtime?.memory?.observation ?? "unavailable"}
          <br />
          加载参数来自已保存的偏好，不代表硬件性能保证。
        </p>
      </div>
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
    ready && !active && !state.operation && !!draft.trim() && !incomplete;
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
          <h1>聊天</h1>
          <p>
            {state.snapshot?.runtime?.selected_model ?? "尚未加载模型"}
            <span className="inline-dot">·</span>会话仅保留在当前窗口
          </p>
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
  const [draft, setDraft] = useState<Preferences>(() => ({
    context_size: settings.context_size,
    threads: settings.threads,
    batch_size: settings.batch_size,
    max_output_tokens: settings.max_output_tokens,
    close_runtime_on_exit: settings.close_runtime_on_exit,
  }));
  const [idle, setIdle] = useState(settings.idle_unload_seconds);
  const [modal, setModal] = useState<"token" | "stop" | null>(null);
  const runtime = state.snapshot?.runtime;
  const running =
    state.snapshot?.connection === "connected" ||
    state.snapshot?.connection === "connecting";
  const stopped = state.snapshot?.connection === "stopped";
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
      detail: "输入、模板与输出共用的 token 预算",
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
          <p>明确何时生效，让本地运行保持可控。</p>
        </div>
      </div>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void controller.saveSettings(draft);
        }}
      >
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
              {runtime?.load_options
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
            disabled={!!state.operation || !!validation}
          >
            {state.operation === "正在保存偏好" && <Spinner />}保存偏好
          </button>
        </div>
      </form>
      <section className="settings-card idle-card">
        <div className="card-heading">
          <div>
            <h2>空闲卸载</h2>
            <p>运行服务配置，必须先停止服务；保存后在下次启动时生效。</p>
          </div>
          <span className="mini-label">独立应用</span>
        </div>
        <div className="idle-controls">
          <label htmlFor="idle">
            空闲等待时间
            <div className="number-field">
              <input
                type="number"
                id="idle"
                min={1}
                max={86400}
                value={Number.isNaN(idle) ? "" : idle}
                onChange={(event) => setIdle(event.target.valueAsNumber)}
              />
              <span>秒</span>
            </div>
          </label>
          <button
            disabled={
              !stopped ||
              !!state.operation ||
              !Number.isSafeInteger(idle) ||
              idle < 1 ||
              idle > 86400
            }
            onClick={() => void controller.saveIdle(idle)}
          >
            应用空闲卸载设置
          </button>
        </div>
        <p className="small-note">
          当前服务配置：{settings.idle_unload_seconds} 秒
          {!stopped ? " · 请先停止运行服务再修改" : ""}
        </p>
      </section>
      <section className="settings-card">
        <div className="card-heading">
          <div>
            <h2>本机 API</h2>
            <p>只在本机使用，由原生端验证连接与管理凭据。</p>
          </div>
          <Icon name="shield" size={22} />
        </div>
        <div className="api-row">
          <span>API 地址</span>
          <output>{state.snapshot?.api_address ?? "未连接"}</output>
        </div>
        <div className="token-row">
          <p>
            复制令牌会将凭据写入系统剪贴板。
            <br />
            <span>请勿粘贴到聊天或分享给他人，使用后及时清除。</span>
          </p>
          <button
            disabled={!state.snapshot?.initialized || !!state.operation}
            onClick={() => setModal("token")}
          >
            <Icon name="copy" size={16} />
            复制 API 令牌
          </button>
        </div>
      </section>
      <section className="settings-card service-card">
        <div>
          <h2>运行服务</h2>
          <p>停止服务会影响所有连接到 Nexa 的客户端。</p>
        </div>
        <button
          className="danger-outline"
          disabled={
            !running || !!state.operation || state.chat_phase !== "idle"
          }
          onClick={() => setModal("stop")}
        >
          <Icon name="power" size={16} />
          停止运行服务
        </button>
      </section>
      {modal && (
        <Modal
          title={
            modal === "token"
              ? "将令牌复制到系统剪贴板？"
              : "停止所有客户端的运行任务？"
          }
          confirm={modal === "token" ? "确认复制" : "停止运行服务"}
          danger={modal === "stop"}
          onCancel={() => setModal(null)}
          onConfirm={() => {
            const action = modal;
            setModal(null);
            void (action === "token"
              ? controller.copyToken()
              : controller.stop());
          }}
        >
          {modal === "token" ? (
            <p>
              其他应用或剪贴板历史可能读取这份凭据。请仅粘贴到你信任的本机客户端，使用后及时清除。
            </p>
          ) : (
            <p>
              这会停止本机运行服务，终止所有客户端的任务并卸载模型。收到清理确认后才会显示已停止。
            </p>
          )}
        </Modal>
      )}
    </>
  );
}
export default function App({
  controller,
  preview = false,
}: {
  controller: DesktopController;
  preview?: boolean;
}) {
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const [page, setPage] = useState<"models" | "chat" | "settings">("models");
  const [draft, setDraft] = useState("");
  useEffect(() => controller.mount(), [controller]);
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.altKey && ["1", "2", "3"].includes(event.key)) {
        event.preventDefault();
        setPage(
          (["models", "chat", "settings"] as const)[Number(event.key) - 1],
        );
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, []);
  const nav = [
    { id: "models", title: "模型", detail: "管理本机模型", icon: "models" },
    { id: "chat", title: "聊天", detail: "验证本地推理", icon: "chat" },
    { id: "settings", title: "设置", detail: "运行与偏好", icon: "settings" },
  ] as const;
  const healthy =
    state.snapshot?.connection === "connected" &&
    state.snapshot.runtime?.state !== "faulted";
  return (
    <div className="app-shell">
      <aside className="sidebar">
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
              onClick={() => setPage(item.id)}
            >
              <Icon name={item.icon} />
              <span>
                <strong>{item.title}</strong>
                <small>{item.detail}</small>
              </span>
              {item.id === "chat" && state.chat_phase !== "idle" && (
                <span className="nav-live" />
              )}
            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <div className="local-card">
            <Icon name="shield" size={18} />
            <strong>留在你的设备上</strong>
            <p>
              模型在本机运行
              <br />
              聊天不会保存到磁盘
            </p>
          </div>
          <div className="sidebar-footer">
            <span>Windows · CPU</span>
            <button
              title="按已保存策略关闭应用"
              aria-label="关闭应用"
              className="icon-button"
              disabled={!!state.operation}
              onClick={() => void controller.close()}
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
            {nav.find((item) => item.id === page)?.title}
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
        <main
          className={
            page === "chat" ? "main-content chat-content" : "main-content"
          }
        >
          <RuntimeBanner state={state} controller={controller} />
          {state.error && (
            <div className="error-banner" role="alert">
              <div>
                <strong>操作未完成</strong>
                <p>{state.error.message}</p>
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
          {state.notice && (
            <div className="success-notice" role="status">
              {state.notice}
            </div>
          )}
          {page === "models" && (
            <ModelsPage
              state={state}
              controller={controller}
              goChat={() => setPage("chat")}
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
        </main>
      </div>
    </div>
  );
}
