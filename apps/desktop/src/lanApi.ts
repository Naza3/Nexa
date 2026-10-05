import type { LanAddressDiscovery, LanApiSettings } from "./types";

export const DEFAULT_LAN_SETTINGS: LanApiSettings = {
  enabled: false,
  listen: null,
  allowed_cidrs: [],
};
export const LAN_CLIENT_LIMIT = 16;

function privateIpv4(value: string): number | null {
  const parts = value.split(".");
  if (parts.length !== 4 || parts.some((part) => !/^(0|[1-9]\d{0,2})$/.test(part) || Number(part) > 255)) return null;
  const [a, b, c, d] = parts.map(Number);
  if (!(a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168))) return null;
  return (a * 2 ** 24 + b * 2 ** 16 + c * 256 + d) >>> 0;
}

/** Mirrors the native boundary for feedback only; the native side remains authoritative. */
export function validateLanSettings(settings: LanApiSettings): string | null {
  if (typeof settings.enabled !== "boolean" || !Array.isArray(settings.allowed_cidrs)) return "局域网配置无效。";
  if (settings.listen !== null) {
    const parts = settings.listen.split(":");
    const port = Number(parts[1]);
    if (parts.length !== 2 || privateIpv4(parts[0]) === null || !/^[1-9]\d{0,4}$/.test(parts[1]) || port > 65535)
      return "请填写具体的私有 IPv4 地址和 1–65535 的端口；不接受公网、回环、通配地址或 IPv6。";
  }
  if (settings.enabled && !settings.listen) return "启用前请填写此电脑在局域网中的私有 IPv4 地址。";
  if (settings.allowed_cidrs.length > LAN_CLIENT_LIMIT || (settings.enabled && settings.allowed_cidrs.length === 0))
    return "启用时须指定 1–16 个客户端 IP 或 CIDR，不能允许任意客户端。";
  const ranges: { first: number; last: number }[] = [];
  for (const cidr of settings.allowed_cidrs) {
    const parts = cidr.split("/");
    const address = privateIpv4(parts[0]);
    const prefix = Number(parts[1]);
    if (parts.length !== 2 || address === null || !/^(2[4-9]|3[0-2])$/.test(parts[1]))
      return "白名单只接受私有 IPv4 或 /24–/32 CIDR，不接受公网、通配或更大的网段。";
    const mask = (0xffffffff << (32 - prefix)) >>> 0;
    const first = (address & mask) >>> 0;
    if (address !== first) return "CIDR 须使用规范网络地址，例如 192.168.1.0/24；单个客户端可直接填写 IP。";
    const last = first + 2 ** (32 - prefix) - 1;
    if (ranges.some((range) => first <= range.last && last >= range.first)) return "白名单存在重复或重叠的 IP / CIDR，请合并后重试。";
    ranges.push({ first, last });
  }
  return null;
}

export function lanSettingsFromDraft(enabled: boolean, host: string, port: string, clients: string): LanApiSettings {
  const allowed_cidrs = clients.trim() ? clients.trim().split(/\r?\n/).map((line) => {
    const entry = line.trim();
    return entry && !entry.includes("/") ? `${entry}/32` : entry;
  }) : [];
  return { enabled, listen: host.trim() ? `${host.trim()}:${port.trim()}` : null, allowed_cidrs };
}

export function lanBaseUrl(settings: LanApiSettings): string | null {
  return settings.enabled && settings.listen && !validateLanSettings(settings) ? `http://${settings.listen}/v1` : null;
}

/** Fail closed on malformed discovery data; never accept public/wildcard suggestions. */
export function validLanAddresses(value: unknown): value is LanAddressDiscovery {
  if (!value || typeof value !== "object" || !("status" in value) || !("addresses" in value) ||
    typeof value.status !== "string" || !["available", "empty", "unsupported"].includes(value.status) || !Array.isArray(value.addresses) ||
    value.addresses.length > 256 || (value.status === "available") !== (value.addresses.length > 0)) return false;
  const seen = new Set<string>();
  return value.addresses.every((entry: unknown) => {
    if (!entry || typeof entry !== "object" || !("interface_index" in entry) || !("interface_name" in entry) || !("address" in entry) ||
      !Number.isInteger(entry.interface_index) || Number(entry.interface_index) < 1 || Number(entry.interface_index) > 0xffffffff ||
      typeof entry.interface_name !== "string" || !entry.interface_name.trim() || entry.interface_name.length > 256 ||
      /\p{Cc}/u.test(entry.interface_name) || typeof entry.address !== "string" || privateIpv4(entry.address) === null) return false;
    const key = `${entry.interface_index}:${entry.address}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}
