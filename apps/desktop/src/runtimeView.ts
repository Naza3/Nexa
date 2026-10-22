import type { Snapshot } from "./types";

/** A presentation projection only. The native snapshot remains authoritative. */
export function runtimeView(snapshot: Snapshot | null) {
  const connected = snapshot?.connection === "connected";
  const runtime = connected ? snapshot.runtime : null;
  const resident = !!runtime?.selected_model && ["ready", "generating"].includes(runtime.state);
  const service = !snapshot ? "unknown" : runtime?.stopping ? "stopping" : snapshot.connection;
  const serviceLabel = ({ unknown: "服务状态待确认", stopped: "服务已停止", connecting: "服务正在启动", connected: "服务运行中", stopping: "服务正在停止", error: "服务连接失效" })[service];
  const modelLabel = !runtime ? (snapshot?.connection === "stopped" ? "无驻留模型" : "驻留状态待确认") : ({ unloaded: "无驻留模型", loading: "正在加载模型", ready: "模型已就绪", generating: "模型正在生成", unloading: "正在释放模型", faulted: "模型运行故障" })[runtime.state];
  const busy = !!runtime && (runtime.stopping || runtime.registry_busy || !!runtime.active_request || runtime.queued_jobs > 0 || ["loading", "generating", "unloading"].includes(runtime.state));
  const localListening = connected && !!snapshot?.api_address;
  const lanListening = connected && runtime?.lan_api?.running === true && runtime.lan_api.enabled && !!runtime.lan_api.listen;
  const lanDegraded = connected && runtime?.lan_api?.enabled === true && !runtime.lan_api.running && !!runtime.lan_api.startup_error;
  // Queue capacity is not currently observable. Do not invent queue fullness or auth success.
  const readiness = !localListening ? "本机 API 未确认在线" : runtime?.stopping ? "服务停止中，暂不可调用" : runtime?.registry_busy ? "服务在线，模型操作中" : !resident ? "服务在线，无就绪驻留模型" : runtime.state === "generating" ? "服务在线，正在生成；新请求仍需准入" : "服务在线，驻留模型可供调用";
  return { runtime, service, serviceLabel, modelLabel, resident, residentId: resident ? runtime.selected_model : null,
    residentName: resident ? runtime.selected_model_display_name ?? runtime.selected_model : null,
    residentOptions: resident ? runtime.load_options : null,
    lastSelectedId: runtime?.selected_model ?? null, lastSelectedName: runtime?.selected_model_display_name ?? runtime?.selected_model ?? null,
    busy, localListening, lanListening, lanDegraded, readiness };
}
export function localBaseUrl(address: string | null | undefined): string | null {
  if (!address) return null;
  try {
    const url = new URL(address);
    const loopback = ["localhost", "[::1]"].includes(url.hostname) || /^127(?:\.\d{1,3}){3}$/.test(url.hostname) && url.hostname.split(".").every((part) => Number(part) <= 255);
    if (url.protocol !== "http:" || !loopback || url.username || url.password || url.search || url.hash || !["/", "/v1", "/v1/"].includes(url.pathname)) return null;
    return `${url.origin}/v1`;
  } catch { return null; }
}
