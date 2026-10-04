import { describe, expect, it, vi } from "vitest";
import { DesktopController } from "../src/controller";
import { DEFAULT_LAN_SETTINGS, lanBaseUrl, lanSettingsFromDraft, validateLanSettings } from "../src/lanApi";
import type { LanApiSettings, Snapshot } from "../src/types";
import { deferred, makeApi, snapshot } from "./fixtures";

const enabled: LanApiSettings = { enabled: true, listen: "192.168.1.20:18081", allowed_cidrs: ["192.168.1.30/32"] };
const stopped = (): Snapshot => ({ ...snapshot(), connection: "stopped", runtime: null });

describe("bounded private LAN configuration", () => {
  it("defaults closed and converts an explicit client IP to /32", () => {
    expect(DEFAULT_LAN_SETTINGS).toEqual({ enabled: false, listen: null, allowed_cidrs: [] });
    expect(validateLanSettings(DEFAULT_LAN_SETTINGS)).toBeNull();
    expect(lanSettingsFromDraft(true, "192.168.1.20", "18081", "192.168.1.30")).toEqual(enabled);
    expect(lanBaseUrl(enabled)).toBe("http://192.168.1.20:18081/v1");
    expect(lanBaseUrl({ ...enabled, enabled: false })).toBeNull();
  });
  it.each(["10.0.0.1:1", "172.16.0.1:65535", "172.31.255.254:18081", "192.168.1.20:18081"])("allows a concrete RFC1918 socket %s", (listen) => {
    expect(validateLanSettings({ ...enabled, listen })).toBeNull();
  });
  it.each([null, "", "0.0.0.0:18081", "127.0.0.1:18081", "8.8.8.8:18081", "172.15.0.1:18081", "172.32.0.1:18081", "169.254.1.1:18081", "[::]:18081", "192.168.01.1:18081", "192.168.1.1:0", "192.168.1.1:65536", "192.168.1.1:1.5", "192.168.1.1:1e3", "192.168.1.1:018081", "192.168.1.1", "*:18081"])("rejects wildcard/public/malformed listen %s", (listen) => {
    expect(validateLanSettings({ ...enabled, listen })).not.toBeNull();
  });
  it.each([[], ["*"], ["0.0.0.0/0"], ["8.8.8.8/32"], ["127.0.0.1/32"], ["10.1.0.0/16"], ["10.1.2.1/24"], ["192.168.1.1/33"], ["192.168.1.30/32", "192.168.1.30/32"], ["192.168.1.0/24", "192.168.1.30/32"], ["192.168.1.0/25", "192.168.1.0/24"], ["192.168.1.00/24"]].map((allowed_cidrs) => ({ allowed_cidrs })))("rejects empty, public, noncanonical or overlapping allowlists $allowed_cidrs", ({ allowed_cidrs }) => {
    expect(validateLanSettings({ ...enabled, allowed_cidrs })).not.toBeNull();
  });
  it("bounds allowlists and supports adjacent canonical private ranges", () => {
    expect(validateLanSettings({ ...enabled, allowed_cidrs: Array.from({ length: 16 }, (_, index) => `10.0.0.${index}/32`) })).toBeNull();
    expect(validateLanSettings({ ...enabled, allowed_cidrs: Array.from({ length: 17 }, (_, index) => `10.0.0.${index}/32`) })).toContain("1–16");
    expect(validateLanSettings({ ...enabled, allowed_cidrs: ["192.168.1.0/25", "192.168.1.128/25"] })).toBeNull();
    expect(validateLanSettings({ ...enabled, enabled: false, listen: "0.0.0.0:80" })).not.toBeNull();
    expect(validateLanSettings(lanSettingsFromDraft(true, "192.168.1.20", "18081", "192.168.1.30\n\n192.168.1.40"))).not.toBeNull();
  });
});

describe("LAN controller isolation and races", () => {
  it("saves without starting, stopping, generating credentials or changing offline inventory", async () => {
    const api = makeApi({ snapshot: vi.fn(async () => stopped()) });
    const controller = new DesktopController(api);
    await controller.refresh();
    const models = controller.getSnapshot().models;
    await controller.saveLanSettings(enabled);
    expect(api.saveLanSettings).toHaveBeenCalledExactlyOnceWith(enabled);
    expect(controller.getSnapshot().models).toBe(models);
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
    expect(api.copyLanToken).not.toHaveBeenCalled();
  });
  it.each(["connected", "connecting", "error"] as const)("refuses %s locally without implicit stop", async (connection) => {
    const api = makeApi({ snapshot: vi.fn(async () => ({ ...snapshot(), connection })) });
    const controller = new DesktopController(api);
    await controller.refresh();
    await controller.saveLanSettings(enabled);
    expect(controller.getSnapshot().error?.code).toBe("runtime_running");
    expect(api.saveLanSettings).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("does not configure an old bridge or initialize a missing runtime", async () => {
    for (const state of [{ ...stopped(), lan_api: undefined }, { ...stopped(), initialized: false }]) {
      const api = makeApi({ snapshot: vi.fn(async () => state) });
      const controller = new DesktopController(api);
      await controller.refresh();
      await controller.saveLanSettings(enabled);
      await controller.copyLanToken();
      expect(api.saveLanSettings).not.toHaveBeenCalled();
      expect(api.start).not.toHaveBeenCalled();
      expect(api.copyLanToken).not.toHaveBeenCalled();
    }
  });
  it("deduplicates saves and preserves authoritative configuration on a native running race", async () => {
    const pending = deferred<Snapshot>();
    const api = makeApi({ snapshot: vi.fn(async () => stopped()), saveLanSettings: vi.fn(() => pending.promise) });
    const controller = new DesktopController(api);
    await controller.refresh();
    const saving = controller.saveLanSettings(enabled);
    await controller.saveLanSettings(enabled);
    expect(api.saveLanSettings).toHaveBeenCalledTimes(1);
    pending.reject({ code: "runtime_running", message: "运行服务已被其他客户端启动，请先显式停止。" });
    await saving;
    expect(controller.getSnapshot().snapshot?.lan_api?.enabled).toBe(false);
    expect(controller.getSnapshot().error?.code).toBe("runtime_running");
    expect(controller.getSnapshot().notice).toBeNull();
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("deduplicates native-only key copies and never stores returned extras", async () => {
    const pending = deferred<{ copied: true }>();
    const api = makeApi({ snapshot: vi.fn(async () => ({ ...stopped(), lan_api: enabled })), copyLanToken: vi.fn(() => pending.promise) });
    const controller = new DesktopController(api);
    await controller.refresh();
    const copying = controller.copyLanToken();
    await controller.copyLanToken();
    expect(api.copyLanToken).toHaveBeenCalledTimes(1);
    pending.resolve({ copied: true, token: "DO-NOT-RETAIN" } as { copied: true });
    await copying;
    expect(controller.getSnapshot().notice).toContain("独立密钥已复制");
    expect(JSON.stringify(controller.getSnapshot())).not.toContain("DO-NOT-RETAIN");
  });
  it("clears success feedback on expired/missing-key failures without exposing diagnostic secrets", async () => {
    const api = makeApi({ snapshot: vi.fn(async () => ({ ...stopped(), lan_api: enabled })) });
    const controller = new DesktopController(api);
    await controller.refresh();
    await controller.copyLanToken();
    vi.mocked(api.copyLanToken).mockRejectedValue({ code: "lan_token_unavailable", message: "expired secret DO-NOT-RETAIN" });
    await controller.copyLanToken();
    expect(controller.getSnapshot().notice).toBeNull();
    expect(controller.getSnapshot().error?.code).toBe("lan_token_unavailable");
    expect(JSON.stringify(controller.getSnapshot())).not.toContain("DO-NOT-RETAIN");
    expect(api.start).not.toHaveBeenCalled();
  });
  it("reports URL clipboard failures without success and copies only the saved URL", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    const descriptor = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    try {
      const controller = new DesktopController(makeApi({ snapshot: vi.fn(async () => ({ ...stopped(), lan_api: enabled })) }));
      await controller.refresh();
      await controller.copyLanBaseUrl();
      expect(writeText).toHaveBeenCalledExactlyOnceWith("http://192.168.1.20:18081/v1");
      writeText.mockRejectedValueOnce(new Error("raw clipboard error"));
      await controller.copyLanBaseUrl();
      expect(controller.getSnapshot().notice).toBeNull();
      expect(controller.getSnapshot().error?.code).toBe("clipboard_unavailable");
      expect(JSON.stringify(controller.getSnapshot())).not.toContain("raw clipboard");
    } finally {
      if (descriptor) Object.defineProperty(navigator, "clipboard", descriptor);
      else Reflect.deleteProperty(navigator, "clipboard");
    }
  });
});
