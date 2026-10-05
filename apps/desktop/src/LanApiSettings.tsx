import { useEffect, useState } from "react";
import { useConfigDraft } from "./configDraft";
import { DraftConflict } from "./Configuration";
import type { DesktopController, ViewState } from "./controller";
import { DEFAULT_LAN_SETTINGS, lanBaseUrl, lanSettingsFromDraft, validateLanSettings } from "./lanApi";
import { Modal } from "./Modal";
import type { LanApiSettings as LanSettings } from "./types";

function draftFields(source: LanSettings) {
  return { enabled: source.enabled, host: source.listen?.split(":")[0] ?? "", port: source.listen?.split(":")[1] ?? "18081", clients: source.allowed_cidrs.join("\n") };
}

export function LanApiSettings({ state, controller }: { state: ViewState; controller: DesktopController }) {
  const snapshot = state.snapshot;
  const configured = snapshot?.configuration?.saved.lan_api ?? snapshot?.lan_api ?? DEFAULT_LAN_SETTINGS;
  const editor = useConfigDraft(draftFields(configured), snapshot?.configuration?.revision, "lan_api");
  const form = { source: configured, ...editor.draft };
  const setForm = (next: typeof form) => editor.setDraft({ enabled: next.enabled, host: next.host, port: next.port, clients: next.clients });
  const { enabled, host, port, clients } = form;
  useEffect(() => { void controller.refreshLanAddresses(); }, [controller]);
  const [selectedInterface, setSelectedInterface] = useState<number | null>(null);
  const addresses = state.lan_addresses?.addresses ?? [];
  const detected = addresses.find((entry) => entry.address === host && (selectedInterface === null || entry.interface_index === selectedInterface));
  const discoveryStatus = state.lan_addresses_loading ? "正在检测本机网卡…"
    : state.lan_addresses_error ? state.lan_addresses_error.message
    : state.lan_addresses?.status === "unsupported" ? "当前平台或开发预览不支持网卡检测，请手动填写 IPv4。"
    : state.lan_addresses?.status === "empty" ? "未检测到可用的私有 IPv4。请检查网络后刷新，或手动填写。"
    : addresses.length ? `检测到 ${addresses.length} 个私有 IPv4，请选择本机用于局域网的网卡。`
    : "尚未检测本机网卡。";
  const [modal, setModal] = useState<"enable" | "copy" | null>(null);
  const supported = !!snapshot?.lan_api;
  const stopped = snapshot?.connection === "stopped";
  const busy = !!state.operation || state.library_phase !== "idle" || state.download_phase !== "idle" || state.chat_phase !== "idle";
  const editable = supported && !!snapshot?.initialized && stopped && !busy;
  const draft = enabled ? lanSettingsFromDraft(true, host, port, clients) : { ...configured, enabled: false };
  const validation = validateLanSettings(draft);
  const changed = JSON.stringify(draft) !== JSON.stringify(configured);
  const baseUrl = lanBaseUrl(configured);
  const actual = snapshot?.runtime?.lan_api;
  const actualRunning = snapshot?.connection === "connected" && actual?.enabled && actual.running && actual.listen;
  const actualLabel = actualRunning ? `正在监听 http://${actual.listen}/v1`
    : stopped ? "未运行（服务已停止）"
    : snapshot?.connection !== "connected" ? "状态待确认"
    : !actual ? "当前服务未报告局域网监听状态"
    : "未运行";

  function disable() {
    setForm({ source: configured, ...draftFields(configured), enabled: false });
    setModal(null);
  }

  return (
    <section className="settings-card lan-settings" aria-labelledby="lan-api-title">
      {editor.conflict && <DraftConflict reset={editor.reset} rebase={editor.rebase} draft={editor.draft} saved={draftFields(configured)} />}
      <div className="toggle-row">
        <div>
          <h2 id="lan-api-title">局域网 API</h2>
          <p>默认关闭。仅用于可信局域网，保存后需显式启动运行服务。</p>
        </div>
        <input type="checkbox" role="switch" className="switch" aria-label="启用局域网 API"
          checked={enabled} disabled={!editable}
          onChange={(event) => event.target.checked ? setModal("enable") : disable()} />
      </div>
      <p className="warning-text lan-risk">使用明文 HTTP，不提供 TLS 加密。API 密钥、对话和回复可能被网络中的其他设备读取或篡改；请仅在你信任的网络和设备间使用。</p>
      <p className="small-note">仅开放 /v1/models 与 /v1/chat/completions。管理接口保持本机回环访问；不会自动修改防火墙或配置 NAT / 端口转发。</p>
      <p className="small-note">局域网客户端只能列出和调用本机已加载的模型。尚未加载或空闲自动卸载后，请回到本机模型页加载；客户端不能远程加载、卸载或切换模型。</p>
      {!supported ? <p className="warning-text">当前桌面版本未提供局域网 API 设置，继续使用本机 API。</p>
        : !snapshot?.initialized ? <p className="warning-text">请先到设置选择“仅初始化配置”，无需先启动再停止服务；此页面不会初始化服务或生成密钥。</p>
        : !stopped && <p className="warning-text">请先显式停止运行服务再修改。保存不会自动中断任务或切换监听地址。</p>}
      {enabled && <div className="lan-fields">
        <div className="lan-discovery">
          <div className="lan-discovery-heading">
            <strong>检测本机网卡</strong>
            <button type="button" disabled={state.lan_addresses_loading} onClick={() => void controller.refreshLanAddresses()}>
              {state.lan_addresses_loading ? "正在检测…" : "刷新网卡地址"}
            </button>
          </div>
          <p role="status" className={state.lan_addresses_error ? "warning-text" : "small-note"}>{discoveryStatus}</p>
          {!!addresses.length && <label className="setting-field" htmlFor="lan-address-choice"><span>选择网卡地址</span>
            <select id="lan-address-choice" disabled={!editable || state.lan_addresses_loading}
              value={detected ? `${detected.interface_index}:${detected.address}` : ""}
              onChange={(event) => {
                const entry = addresses.find((item) => `${item.interface_index}:${item.address}` === event.target.value);
                if (entry && editable) { setSelectedInterface(entry.interface_index); setForm({ ...form, host: entry.address }); }
              }}>
              <option value="" disabled>请选择检测到的网卡和 IPv4</option>
              {addresses.map((entry) => <option key={`${entry.interface_index}:${entry.address}`} value={`${entry.interface_index}:${entry.address}`}>
                {entry.interface_name} · {entry.address}
              </option>)}
            </select>
          </label>}
          <p className="small-note">仅列出本机观察到的私有 IPv4，可能包含 VPN 或虚拟网卡，不代表网络可信或客户端可达。检测不会修改配置；选择后仍需在服务停止时保存。</p>
        </div>
        <div className="settings-grid">
          <label className="setting-field" htmlFor="lan-host"><span>本机局域网 IPv4</span>
            <small>可从上方选择，也可手动填写此电脑的具体私有地址</small>
            <input id="lan-host" type="text" value={host} maxLength={15} placeholder="192.168.1.20" autoComplete="off" spellCheck={false}
              disabled={!editable} onChange={(event) => { setSelectedInterface(null); setForm({ ...form, host: event.target.value }); }} />
          </label>
          <label className="setting-field" htmlFor="lan-port"><span>局域网端口</span>
            <small>1–65535；为此监听地址选择未被占用的端口</small>
            <input id="lan-port" type="text" inputMode="numeric" maxLength={5} value={port} autoComplete="off"
              disabled={!editable} onChange={(event) => setForm({ ...form, port: event.target.value })} />
          </label>
        </div>
        <label className="setting-field" htmlFor="lan-clients"><span>允许的客户端 IP / CIDR</span>
          <small>每行一个，限 1–16 条私有 IPv4 或规范 /24–/32 网段；不得为空、重复、重叠或使用通配。单个 IP 会保存为 /32。</small>
          <textarea id="lan-clients" rows={4} maxLength={512} value={clients} autoComplete="off" spellCheck={false}
            placeholder={"192.168.1.30\n192.168.2.0/24"} disabled={!editable} onChange={(event) => setForm({ ...form, clients: event.target.value })} />
        </label>
      </div>}
      <div className="lan-status" aria-label="局域网 API 状态">
        <p>已保存配置：{configured.enabled ? `启用 · ${configured.listen}` : "关闭"}</p>
        <p>实际监听：{actualLabel}</p>
      </div>
      {validation && <p role="alert" className="warning-text">{validation}</p>}
      <div className="save-row">
        <span className="muted">{changed ? "有未保存的更改" : "配置与已保存内容一致"}</span>
        <button className="primary" disabled={!editable || !!validation || !changed || editor.conflict}
          onClick={() => void (snapshot?.configuration ? controller.saveConfiguration({ expected_revision: editor.baseRevision!, update: { kind: "lan_api", lan_api: draft } }) : controller.saveLanSettings(draft))}>保存局域网 API 设置</button>
      </div>
      {baseUrl && <div className="lan-client-url">
        <div className="api-row"><span>客户端 Base URL</span><output aria-label="局域网客户端 Base URL">{baseUrl}</output></div>
        <button disabled={busy} onClick={() => void controller.copyLanBaseUrl()}>复制局域网 Base URL</button>
        <p className="small-note">使用已保存的 IPv4、端口与 /v1 后缀。已保存不代表正在监听；请以“实际监听”为准，并确认防火墙允许可信客户端访问。</p>
      </div>}
      <div className="token-row">
        <p>LAN 使用独立密钥，不能使用本机管理令牌。<br />首次启动已启用 LAN 的服务后才会生成；保存配置不会生成。<br />复制后仅粘贴到白名单内可信客户端，使用后及时清除剪贴板。</p>
        <button disabled={!supported || !snapshot?.initialized || !configured.enabled || busy}
          onClick={() => setModal("copy")}>复制局域网 API 密钥</button>
      </div>
      {modal && <Modal title={modal === "enable" ? "允许可信局域网通过 HTTP 访问？" : "将局域网独立密钥复制到剪贴板？"}
        confirm={modal === "enable" ? "了解风险，编辑配置" : "确认复制局域网密钥"}
        onCancel={() => setModal(null)} onConfirm={() => {
          const action = modal;
          setModal(null);
          if (action === "enable") { if (editable) setForm({ ...form, enabled: true }); }
          else void controller.copyLanToken();
        }}>
        {modal === "enable" ? <p>密钥、对话和回复通过未加密的 HTTP 传输，可能遭到窃听或篡改。获准客户端持密钥可使用模型。仅使用可信网络，避免公网或端口转发；地址与白名单检查不提供加密保护。确认后仍须保存配置并显式启动服务。</p>
          : <p>其他应用或剪贴板历史可能读取这份独立密钥。请仅粘贴到已允许的可信局域网客户端，使用后及时清除。密钥不会显示在页面。</p>}
      </Modal>}
    </section>
  );
}
