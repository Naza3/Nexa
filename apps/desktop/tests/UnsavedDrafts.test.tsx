import { openSettingsGroups } from "./navigation";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DraftStore, DRAFT_BYTES, DRAFT_STORAGE_KEY, readUnsavedDrafts, validDraftValue } from "../src/unsavedDrafts";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { configuredSnapshot, revision } from "./configurationFixtures";
import { makeApi } from "./fixtures";
const source = { global_defaults: { context_size: 4096, threads: null, batch_size: 512 } };
const draft = { global_defaults: { context_size: 8192, threads: null, batch_size: 512 } };
function saved() { return { source, draft, revision: revision(), conflict: false }; }
describe("bounded untrusted unsaved-draft recovery", () => {
  it("recovers only to pending state, never active or backend-saved config", () => {
    const store = new DraftStore(); store.set("global_defaults", saved()); const reopened = new DraftStore(); expect(reopened.size).toBe(0); expect(reopened.pending.get("global_defaults")).toMatchObject({ source, draft, revision: revision() }); reopened.restorePending(); expect(reopened.pending.size).toBe(0); expect(reopened.get("global_defaults")?.draft).toEqual(draft);
  });
  it.each([
    { key: "token", source: "secret", draft: "secret" },
    { key: "local_api", source: { listen: "127.0.0.1:18181" }, draft: { listen: "C:\\private\\secret" } },
    { key: "ui_preferences", source: { close_runtime_on_exit: false, download_source: "modelscope" }, draft: { close_runtime_on_exit: false, download_source: "modelscope", api_key: "secret" } },
    { key: "model_profile:../path", source: { context_size: null, threads: null, batch_size: null }, draft: { context_size: 2048, threads: null, batch_size: null } },
    { key: "lan_api", source: { enabled: false, host: "", port: "18081", clients: "" }, draft: { enabled: true, host: "password", port: "18081", clients: "" } },
  ])("rejects non-allowlisted recovery content: $key", (entry) => {
    localStorage.setItem(DRAFT_STORAGE_KEY, JSON.stringify({ version: 1, entries: [{ ...entry, revision: revision() }] })); const store = new DraftStore(); expect(store.pending.size).toBe(0); expect(store.warning).toContain("未采用");
  });
  it.each(["not json", JSON.stringify({ version: 99, entries: [] }), "x".repeat(DRAFT_BYTES + 1)])("rejects corrupted, unknown or oversized caches", (raw) => {
    localStorage.setItem(DRAFT_STORAGE_KEY, raw); expect(() => readUnsavedDrafts()).toThrow();
  });
  it("limits groups and retains unsaved memory when cache cannot be updated", () => {
    const store = new DraftStore(); for (let i = 0; i < 65; i++) store.set(`model_profile:m${i}`, { source: { context_size: null, threads: null, batch_size: null }, draft: { context_size: 2048, threads: null, batch_size: null }, revision: revision(), conflict: false }); expect(store.size).toBe(65); expect(store.warning).toContain("未能保存"); expect(new TextEncoder().encode(localStorage.getItem(DRAFT_STORAGE_KEY)!).length).toBeLessThanOrEqual(DRAFT_BYTES);
  });
  it("does not persist paths, arbitrary fields, source secrets or oversized addresses", () => {
    expect(validDraftValue("local_api", { listen: "https://host?token=secret" })).toBe(false); expect(validDraftValue("request_defaults", { max_output_tokens: 512, temperature: 0.7, top_p: 0.9, prompt: "private" })).toBe(false);
  });
  it("keeps invalid blank numeric drafts visibly invalid after recovery", () => {
    const store = new DraftStore(); store.set("verification_policy", { source: 600, draft: Number.NaN, revision: revision(), conflict: false }); const restored = readUnsavedDrafts().get("verification_policy"); expect(Number.isNaN(restored?.draft)).toBe(true);
  });
  it("shows persistence failure and refuses to claim discard succeeded when storage rejects removal", () => {
    const store = new DraftStore(); const set = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("quota"); }); store.set("global_defaults", saved()); expect(store.warning).toContain("关闭窗口可能丢失"); set.mockRestore();
    const remove = vi.spyOn(Storage.prototype, "removeItem").mockImplementation(() => { throw new Error("blocked"); }); expect(store.discardAll()).toBe(false); expect(store.size).toBe(1); remove.mockRestore(); expect(store.discardAll()).toBe(true); expect(store.size).toBe(0);
  });
  it("reopening needs explicit restore, keeps the new backend value, and marks old-revision drafts conflicted", async () => {
    const store = new DraftStore(); store.set("global_defaults", saved()); const value = configuredSnapshot(true); value.configuration!.revision = revision("b"); value.configuration!.saved.global_defaults.context_size = 16384; const api = makeApi({ snapshot: vi.fn(async () => value), configurationSave: vi.fn() }); render(<App initialPage="settings" controller={new DesktopController(api)} />);
    await screen.findByRole("button", { name: "恢复未保存草稿" }); expect(await screen.findByRole("spinbutton", { name: "默认上下文长度" })).toHaveValue(16384); expect(api.configurationSave).not.toHaveBeenCalled(); fireEvent.click(screen.getByRole("button", { name: "恢复未保存草稿" })); expect(screen.getByRole("spinbutton", { name: "默认上下文长度" })).toHaveValue(8192); openSettingsGroups(); expect(screen.getByText("配置版本已变化")).toBeVisible(); expect(screen.getByRole("button", { name: "保存全局默认值" })).toBeDisabled(); expect(api.configurationSave).not.toHaveBeenCalled(); expect(api.loadModel).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled();
  });
  it("confirmed discard-and-close clears the recovery cache; cancellation keeps it", async () => {
    const api = makeApi({ snapshot: vi.fn(async () => configuredSnapshot(true)) }); render(<App initialPage="settings" controller={new DesktopController(api)} />); fireEvent.change(await screen.findByRole("spinbutton", { name: "默认上下文长度" }), { target: { value: "8192" } }); expect(localStorage.getItem(DRAFT_STORAGE_KEY)).toContain("8192"); fireEvent.click(screen.getByRole("button", { name: "关闭窗口并保留服务" })); fireEvent.click(screen.getByRole("button", { name: "取消" })); expect(api.close).not.toHaveBeenCalled(); expect(localStorage.getItem(DRAFT_STORAGE_KEY)).toContain("8192"); fireEvent.click(screen.getByRole("button", { name: "关闭窗口并保留服务" })); await act(async () => fireEvent.click(screen.getByRole("button", { name: "放弃草稿并关闭" }))); await waitFor(() => expect(api.close).toHaveBeenCalledTimes(1)); expect(localStorage.getItem(DRAFT_STORAGE_KEY)).toBeNull();
  });
});
