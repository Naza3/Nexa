import { describe, expect, it, vi } from "vitest";
import { DesktopController } from "../src/controller";
import { validLanAddresses } from "../src/lanApi";
import type { LanAddressDiscovery } from "../src/types";
import { deferred, makeApi } from "./fixtures";

const address = { interface_index: 2, interface_name: "以太网", address: "192.168.1.20" };
const available: LanAddressDiscovery = { status: "available", addresses: [address] };

describe("bounded LAN address discovery", () => {
  it.each([available, { status: "empty", addresses: [] }, { status: "unsupported", addresses: [] }])("accepts complete results: $status", (value) => {
    expect(validLanAddresses(value)).toBe(true);
  });
  it.each([
    null, {}, { status: ["empty"], addresses: [] }, { status: "available", addresses: [] }, { status: "empty", addresses: [address] },
    { status: "unknown", addresses: [] }, { status: "available", addresses: Array(257).fill(address) },
    ...["8.8.8.8", "127.0.0.1", "0.0.0.0", "192.168.01.2", "::1", "169.254.1.2"].map((value) => ({ status: "available", addresses: [{ ...address, address: value }] })),
    ...[0, -1, 1.1, 2 ** 32, "2"].map((value) => ({ status: "available", addresses: [{ ...address, interface_index: value }] })),
    ...["", " ", "a\nsecret", "a".repeat(257)].map((value) => ({ status: "available", addresses: [{ ...address, interface_name: value }] })),
    { status: "available", addresses: [address, address] },
  ])("rejects invalid observations without retaining suggestions: %#", (value) => {
    expect(validLanAddresses(value)).toBe(false);
  });
  it("deduplicates an in-flight read and does not invoke configuration, credentials, or lifecycle actions", async () => {
    const result = deferred<LanAddressDiscovery>();
    const api = makeApi({ lanAddresses: vi.fn(() => result.promise) });
    const controller = new DesktopController(api);
    const first = controller.refreshLanAddresses(); const second = controller.refreshLanAddresses();
    expect(first).toBe(second);
    await Promise.resolve(); expect(api.lanAddresses).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().lan_addresses_loading).toBe(true);
    result.resolve(available); await first;
    expect(controller.getSnapshot().lan_addresses).toEqual(available);
    expect(controller.getSnapshot().lan_addresses_loading).toBe(false);
    expect(controller.getSnapshot().snapshot).toBeNull();
    for (const call of [api.snapshot, api.start, api.stop, api.saveLanSettings, api.copyLanToken]) expect(call).not.toHaveBeenCalled();
  });
  it.each(["failed", "busy", "timeout", "invalid", "limit"])("clears stale candidates and hides raw %s errors, then permits retry", async (suffix) => {
    const api = makeApi({ lanAddresses: vi.fn().mockResolvedValueOnce(available).mockRejectedValueOnce({ code: `lan_address_discovery_${suffix}`, message: "secret-adapter-name" }).mockResolvedValueOnce(available) });
    const controller = new DesktopController(api);
    await controller.refreshLanAddresses(); await controller.refreshLanAddresses();
    expect(controller.getSnapshot().lan_addresses).toBeNull();
    expect(controller.getSnapshot().lan_addresses_error?.code).toBe(`lan_address_discovery_${suffix}`);
    expect(JSON.stringify(controller.getSnapshot())).not.toContain("secret-adapter-name");
    await controller.refreshLanAddresses();
    expect(controller.getSnapshot().lan_addresses_error).toBeNull();
    expect(controller.getSnapshot().lan_addresses).toEqual(available);
  });
  it("permits retry after a synchronous adapter failure", async () => {
    const api = makeApi({ lanAddresses: vi.fn().mockImplementationOnce(() => { throw new Error("failure"); }).mockResolvedValueOnce(available) });
    const controller = new DesktopController(api);
    await controller.refreshLanAddresses(); await controller.refreshLanAddresses();
    expect(api.lanAddresses).toHaveBeenCalledTimes(2);
    expect(controller.getSnapshot().lan_addresses).toEqual(available);
  });
  it("reports malformed native results as unavailable rather than displaying them", async () => {
    const api = makeApi({ lanAddresses: vi.fn().mockResolvedValue({ status: "available", addresses: [{ ...address, address: "8.8.8.8" }] }) });
    const controller = new DesktopController(api); await controller.refreshLanAddresses();
    expect(controller.getSnapshot().lan_addresses).toBeNull();
    expect(controller.getSnapshot().lan_addresses_error?.code).toBe("lan_address_discovery_invalid");
  });
});
