import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ErrorDetails } from "../src/ErrorDetails";
import { deferred } from "./fixtures";
afterEach(() => vi.unstubAllGlobals());
it("copies only controlled diagnostics and deduplicates pending clicks", async () => {
  const wait = deferred<void>(); const write = vi.fn<(text: string) => Promise<void>>(() => wait.promise);
  vi.stubGlobal("navigator", { clipboard: { writeText: write } });
  render(<ErrorDetails error={{ code: "worker_lost", message: "Bearer PRIVATE C:\\Users\\Alice private prompt reply", request_id: "private-id" }} />);
  const button = screen.getByRole("button", { name: "复制诊断" });
  fireEvent.click(button); fireEvent.click(button);
  expect(write).toHaveBeenCalledTimes(1);
  expect(write.mock.calls[0][0]).toContain("worker_lost");
  expect(write.mock.calls[0][0]).not.toMatch(/PRIVATE|Alice|private|reply/);
  expect(screen.getByRole("button", { name: "正在复制诊断…" })).toBeDisabled();
  await act(async () => wait.resolve());
  expect(screen.getByRole("status")).toHaveTextContent("诊断已复制");
});
it("keeps the error visible and explains clipboard failures", async () => {
  vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn().mockRejectedValue(new Error("private")) } });
  render(<ErrorDetails error={{ code: "future_error", message: "private" }} />);
  fireEvent.click(screen.getByRole("button", { name: "复制诊断" }));
  expect(await screen.findByRole("status")).toHaveTextContent("复制失败，请手动复制上方诊断码");
  expect(screen.getByRole("button", { name: "复制诊断" })).toBeEnabled();
});
