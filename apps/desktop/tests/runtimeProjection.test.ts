import { describe, expect, it } from "vitest";
import { localBaseUrl, runtimeView } from "../src/runtimeView";
import { runtime, snapshot } from "./fixtures";
import type { RuntimeStatus } from "../src/types";
describe("authoritative runtime projection", () => {
  it.each(["unloaded", "loading", "unloading", "faulted"] as RuntimeStatus["state"][])("never calls selected model resident in %s", (state) => {
    const value = snapshot(); value.runtime = { ...runtime(), state };
    const view = runtimeView(value);
    expect(view.resident).toBe(false); expect(view.residentId).toBeNull(); expect(view.residentOptions).toBeNull(); expect(view.lastSelectedId).toBe("qwen");
  });
  it.each(["ready", "generating"] as RuntimeStatus["state"][])("marks only %s as resident without guaranteeing admission", (state) => {
    const value = snapshot(); value.runtime!.state = state;
    const view = runtimeView(value); expect(view.resident).toBe(true); expect(view.localListening).toBe(true); expect(view.readiness).not.toContain("保证");
  });
  it.each(["stopped", "connecting", "error"] as const)("ignores stale runtime and listeners while %s", (connection) => {
    const value = snapshot(); value.connection = connection; value.runtime!.lan_api = { enabled: true, running: true, listen: "192.168.1.2:18181" };
    const view = runtimeView(value); expect(view.resident).toBe(false); expect(view.localListening).toBe(false); expect(view.lanListening).toBe(false); expect(view.runtime).toBeNull();
  });
  it("reports releasing and service stopping separately", () => {
    const value = snapshot(); value.runtime!.state = "unloading"; value.runtime!.stopping = true;
    expect(runtimeView(value)).toMatchObject({ service: "stopping", modelLabel: "正在释放模型", resident: false });
  });
  it.each(["http://127.0.0.1:18181", "http://127.0.0.1:18181/v1/"])("normalizes a verified loopback URL %s", (value) => expect(localBaseUrl(value)).toBe("http://127.0.0.1:18181/v1"));
  it.each(["https://evil.test", "http://u:p@127.0.0.1:18181", "http://127.0.0.1:18181?token=secret", "http://127.0.0.1:18181/other", "bad"])("refuses unsafe/unrelated copied address %s", (value) => expect(localBaseUrl(value)).toBeNull());
});
