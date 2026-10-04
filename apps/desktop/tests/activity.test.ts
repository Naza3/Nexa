import { describe, expect, it } from "vitest";
import { ACTIVITY_STORAGE_KEY, persistActivitySummaries, readActivitySummaries, recordActivity } from "../src/activity";
import type { Activity } from "../src/activity";
const item = (id: number, status: Activity["status"] = "completed"): Activity => ({ id: `model:${id}`, kind: "model", label: "model name", status, updated_at: 1791104400000 + id, detail: "private prompt C:\\User\\path Bearer SECRET generated reply", error: { code: "worker_failed", message: "secret error" } });
describe("bounded activity summaries", () => {
  it("bounds terminal history while retaining an older active task", () => {
    let records = [item(0, "running")]; for (let i = 1; i < 100; ++i) records = recordActivity(records, item(i));
    expect(records.filter((value) => value.status === "completed")).toHaveLength(32); expect(records.some((value) => value.id === "model:0")).toBe(true);
  });
  it("updates one identity rather than appending duplicate progress", () => {
    const value = recordActivity([item(1, "running")], item(1)); expect(value).toHaveLength(1); expect(value[0].status).toBe("completed");
  });
  it("persists only allowlisted metadata, never names, paths, prompts, outputs or native result DTOs", () => {
    persistActivitySummaries([item(1)]); const text = localStorage.getItem(ACTIVITY_STORAGE_KEY)!;
    for (const value of ["SECRET", "private", "prompt", "reply", "model name", "secret error", "User"]) expect(text).not.toContain(value);
    expect(JSON.parse(text)[0]).toEqual({ id: "model:1", kind: "model", status: "completed", updated_at: 1791104400001, error_code: "worker_failed" });
  });
  it("reopening never assumes an interrupted task finished or offers native replay", () => {
    persistActivitySummaries([item(1, "running")]); const [value] = readActivitySummaries(); expect(value.status).toBe("recovery"); expect(value.detail).toContain("不会自动重放"); expect(value.model).toBeUndefined(); expect(value.download).toBeUndefined();
  });
  it.each(["invalid", "[{}]", "[null]", "x".repeat(32769)])("ignores malformed or oversized history", (text) => { localStorage.setItem(ACTIVITY_STORAGE_KEY, text); expect(readActivitySummaries()).toEqual([]); });
  it("handles unavailable storage without breaking in-memory results", () => { const set = localStorage.setItem; Object.defineProperty(localStorage, "setItem", { value: () => { throw new Error("blocked"); }, configurable: true }); expect(() => persistActivitySummaries([item(1)])).not.toThrow(); Object.defineProperty(localStorage, "setItem", { value: set, configurable: true }); });
});
