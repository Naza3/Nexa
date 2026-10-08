import { useEffect, useRef, useState } from "react";
import type { AutostartSnapshot, DesktopApi } from "./types";

function checkedSnapshot(value: AutostartSnapshot): AutostartSnapshot {
  if (typeof value?.registered !== "boolean" || typeof value.current_executable !== "boolean") throw new Error("invalid autostart state");
  return value;
}
function failureMessage(error: unknown, fallback: string): string {
  return error && typeof error === "object" && "code" in error && error.code === "autostart_path_too_long"
    ? "当前 Nexa 路径过长，超过 Windows 启动项命令的 260 字符限制。请将完整 Nexa 目录移到更短路径，重新打开后再启用开机启动。"
    : fallback;
}

export function AutostartSettings({ api, closing }: { api: DesktopApi; closing: boolean }) {
  const supported = !!api.autostartGet && !!api.autostartSet;
  const [snapshot, setSnapshot] = useState<AutostartSnapshot | null>(null);
  const [busy, setBusy] = useState(supported);
  const [error, setError] = useState<string | null>(null);
  const epoch = useRef<object>({});
  useEffect(() => {
    const current = {}; epoch.current = current;
    if (api.autostartGet && api.autostartSet) {
      void api.autostartGet().then(checkedSnapshot).then((value) => {
        if (epoch.current === current) { setSnapshot(value); setError(null); }
      }).catch((failure) => {
        if (epoch.current === current) setError(failureMessage(failure, "无法读取开机启动状态，请重新读取后再修改。"));
      }).finally(() => { if (epoch.current === current) setBusy(false); });
    }
    return () => { epoch.current = {}; };
  }, [api]);
  const read = async () => {
    if (!api.autostartGet || busy || closing) return;
    const current = {}; epoch.current = current;
    setBusy(true);
    try { const value = checkedSnapshot(await api.autostartGet()); if (epoch.current === current) { setSnapshot(value); setError(null); } }
    catch (failure) { if (epoch.current === current) { setSnapshot(null); setError(failureMessage(failure, "无法读取开机启动状态，请重新读取后再修改。")); } }
    finally { if (epoch.current === current) setBusy(false); }
  };
  const save = async (enabled: boolean) => {
    if (!api.autostartSet || busy || closing || !snapshot) return;
    const current = {}; epoch.current = current;
    setBusy(true);
    try {
      const value = checkedSnapshot(await api.autostartSet(enabled));
      if (value.registered !== enabled || enabled && !value.current_executable) throw new Error("unconfirmed autostart state");
      if (epoch.current === current) { setSnapshot(value); setError(null); }
    } catch (failure) {
      if (epoch.current === current) { setSnapshot(null); setError(failureMessage(failure, "开机启动修改失败或结果未确认。请重新读取状态；不会将未确认的修改显示为已保存。")); }
    } finally { if (epoch.current === current) setBusy(false); }
  };
  return <section className="settings-card" aria-label="开机启动设置">
    <div className="toggle-row"><div><h2>开机启动</h2><p>登录 Windows 后自动打开 Nexa，默认关闭。此选项与关闭到托盘独立，启动时显示主窗口。</p></div>
      <input className="switch" type="checkbox" role="switch" aria-label="开机启动" checked={snapshot?.registered ?? false} disabled={!supported || !snapshot || busy || closing} onChange={(event) => void save(event.target.checked)} />
    </div>
    <p className="small-note">{!supported ? "当前桌面版本不支持开机启动设置。" : busy ? "正在确认 Windows 启动项…" : snapshot ? snapshot.registered ? "已登记当前用户的登录启动项。Windows 的启动应用设置也可能单独禁用此项。" : "未登记开机启动项。" : "启动状态尚未确认。"}</p>
    {snapshot?.registered && !snapshot.current_executable && <p className="warning-text">启动项指向其他位置的 Nexa。<button disabled={busy || closing} onClick={() => void save(true)}>更新为当前 Nexa</button></p>}
    {error && <p role="alert" className="warning-text">{error}</p>}
    {supported && <button disabled={busy || closing} onClick={() => void read()}>重新读取启动状态</button>}
  </section>;
}
