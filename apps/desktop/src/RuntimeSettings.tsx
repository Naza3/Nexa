import { useState } from "react";
import type { DesktopController, ViewState } from "./controller";
import { DEFAULT_VERIFICATION_SECONDS, MAX_VERIFICATION_SECONDS, MIN_VERIFICATION_SECONDS, validateIdleSeconds, validateVerificationSeconds } from "./runtimeSettingsValues";

type Props = { state: ViewState; controller: DesktopController };
function settingsAccess(state: ViewState) {
  const busy = !!state.operation || state.library_phase !== "idle" || state.download_phase !== "idle" || state.chat_phase !== "idle";
  const reason = !state.snapshot?.initialized
    ? "请先通过左侧服务按钮显式初始化运行服务，再停止服务后配置；保存不会初始化或启动服务。"
    : state.snapshot.connection !== "stopped"
      ? "请先显式停止运行服务再修改；不会自动中断当前任务。"
      : busy ? "有其他操作正在进行，请等待完成或取消后再修改。" : null;
  return { editable: !reason, reason };
}

export function IdleUnloadSettings({ state, controller }: Props) {
  const settings = state.snapshot!.settings;
  const supported = typeof settings.idle_unload_enabled === "boolean";
  const savedEnabled = settings.idle_unload_enabled ?? true;
  const [enabled, setEnabled] = useState(savedEnabled);
  const [seconds, setSeconds] = useState(settings.idle_unload_seconds);
  const access = settingsAccess(state);
  const changed = enabled !== savedEnabled || seconds !== settings.idle_unload_seconds;
  const validation = validateIdleSeconds(seconds);
  const savedIntervalInvalid = !!validateIdleSeconds(settings.idle_unload_seconds);
  function reset() { setEnabled(savedEnabled); setSeconds(settings.idle_unload_seconds); }

  return <section className="settings-card idle-card" aria-labelledby="idle-title">
    <div className="card-heading"><div><h2 id="idle-title">空闲自动卸载</h2>
      <p>模型空闲时释放内存。必须先停止服务，保存后在下次显式启动运行服务时生效。</p></div><span className="mini-label">独立保存</span></div>
    <div className="toggle-row runtime-setting-toggle">
      <div><h3>不自动卸载</h3><p>开启后，模型在空闲时继续驻留内存，直到显式卸载、停止服务或进程退出。</p></div>
      <input type="checkbox" role="switch" className="switch" aria-label="不自动卸载" checked={!enabled}
        disabled={!access.editable || !supported} onChange={(event) => {
          setEnabled(!event.target.checked);
          if (event.target.checked && validation && !savedIntervalInvalid) setSeconds(settings.idle_unload_seconds);
        }} />
    </div>
    <div className="idle-controls">
      <label htmlFor="idle">空闲等待时间<div className="number-field">
        <input type="number" id="idle" min={1} max={86400} step={1} value={Number.isNaN(seconds) ? "" : seconds}
          disabled={!access.editable || (!enabled && !savedIntervalInvalid)} aria-describedby="idle-range" aria-invalid={!!validation}
          onChange={(event) => setSeconds(event.target.valueAsNumber)} /><span>秒</span>
      </div></label>
    </div>
    <p id="idle-range" className="small-note">1–86400 秒。启用“不自动卸载”会保留有效的等待时间，重新启用自动卸载后使用。</p>
    <p className="small-note">关闭自动卸载不会阻止电脑关机或睡眠，也不保证服务持续在线；驻留模型会继续占用内存。</p>
    <p className="small-note" aria-label="已保存空闲卸载配置">已保存配置：{savedEnabled ? `空闲 ${settings.idle_unload_seconds} 秒后自动卸载` : `不自动卸载（保留等待时间 ${settings.idle_unload_seconds} 秒）`}</p>
    {savedIntervalInvalid && <p className="warning-text">旧配置的等待时间超出当前可保存范围，原值保持不变。请先改为 1–86400 秒，再保存启用或停用设置。</p>}
    {!supported && <p className="warning-text">当前桌面版本未提供“不自动卸载”开关，沿用空闲自动卸载；可继续单独调整等待时间。</p>}
    {access.reason && <p className="warning-text">{access.reason}</p>}
    {validation && <p role="alert" className="warning-text">{validation}</p>}
    <div className="save-row runtime-settings-save">
      <span className="muted">{changed ? "有未保存的更改" : "配置与已保存内容一致"}</span>
      <div className="runtime-settings-actions">
        <button disabled={!access.editable || !changed} onClick={reset}>取消空闲卸载更改</button>
        <button disabled={!access.editable || !changed || !!validation}
          onClick={() => void controller.saveIdle(seconds, supported ? enabled : undefined)}>应用空闲卸载设置</button>
      </div>
    </div>
  </section>;
}

export function VerificationTimeoutSettings({ state, controller }: Props) {
  const saved = state.snapshot!.settings.model_verification_timeout_seconds;
  const supported = typeof saved === "number";
  const configured = saved ?? DEFAULT_VERIFICATION_SECONDS;
  const [seconds, setSeconds] = useState(configured);
  const access = settingsAccess(state);
  const editable = access.editable && supported;
  const changed = seconds !== configured;
  const validation = validateVerificationSeconds(seconds);

  return <section className="settings-card idle-card" aria-labelledby="verification-title">
    <div className="card-heading"><div><h2 id="verification-title">模型文件校验超时</h2>
      <p>高级设置。必须先停止服务；保存后用于后续新校验，正在执行的操作不会改动计时。</p></div><span className="mini-label">独立保存</span></div>
    <p className="small-note">适用于全量扫描、选中文件添加、下载完成后的登记校验，以及加载前的外部文件校验。一次校验操作内的所有文件共用时限，不是每个文件单独计时。</p>
    <p className="small-note">此值不改变网络下载传输、原生模型加载或基础短文本生成的独立超时。加载前校验在下次显式启动运行服务后使用新值。</p>
    <div className="idle-controls"><label htmlFor="verification-timeout">模型文件校验时间上限<div className="number-field">
      <input type="number" id="verification-timeout" min={MIN_VERIFICATION_SECONDS} max={MAX_VERIFICATION_SECONDS} step={1}
        value={Number.isNaN(seconds) ? "" : seconds} disabled={!editable} aria-describedby="verification-range" aria-invalid={!!validation}
        onChange={(event) => setSeconds(event.target.valueAsNumber)} /><span>秒</span>
    </div></label></div>
    <p id="verification-range" className="small-note">30–7200 秒，默认 300 秒。大文件或较慢磁盘可能需要更长时间；超时或取消不会跳过校验并登记模型。</p>
    <p className="small-note" aria-label="已保存模型文件校验超时">{supported ? `已保存配置：${configured} 秒` : "旧版本默认：300 秒"}</p>
    {!supported && <p className="warning-text">当前桌面版本未提供模型文件校验超时设置，请更新桌面应用。</p>}
    {access.reason && <p className="warning-text">{access.reason}</p>}
    {validation && <p role="alert" className="warning-text">{validation}</p>}
    <div className="save-row runtime-settings-save">
      <span className="muted">{changed ? "有未保存的更改" : "配置与已保存内容一致"}</span>
      <div className="runtime-settings-actions">
        <button disabled={!editable || !changed} onClick={() => setSeconds(configured)}>取消校验超时更改</button>
        <button disabled={!editable || !changed || !!validation} onClick={() => void controller.saveVerificationTimeout(seconds)}>保存模型文件校验超时</button>
      </div>
    </div>
  </section>;
}
