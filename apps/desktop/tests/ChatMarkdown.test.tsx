import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ChatMarkdown } from "../src/ChatMarkdown";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { ChatBatch, DesktopApi } from "../src/types";
import { deferred, makeApi } from "./fixtures";

afterEach(() => vi.unstubAllGlobals());
const completion = { type: "completed" as const, finish_reason: "stop" as const, usage: { prompt_tokens: 10, completion_tokens: 3, total_tokens: 13 } };
const batch = (text: string, terminal = false): ChatBatch => ({ request_id: "request-1", terminal, events: [...(text ? [{ type: "delta" as const, text }] : []), ...(terminal ? [completion] : [])] });
async function mountChat(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi(overrides), controller = new DesktopController(api);
  render(<App controller={controller} initialPage="chat" />);
  await waitFor(() => expect(controller.getSnapshot().snapshot?.connection).toBe("connected"));
  return { api, controller };
}
function send(text: string) {
  fireEvent.change(screen.getByRole("textbox", { name: "输入消息" }), { target: { value: text } });
  fireEvent.click(screen.getByRole("button", { name: "发送" }));
}

describe("safe chat Markdown with the production parser", () => {
  it("renders headings, emphasis, nested and ordered lists, quotes, rules, inline and fenced code", () => {
    const { container } = render(<ChatMarkdown content={'# 标题\n\n## 二级\n\n**粗体**与*斜体*和`a < b`\n\n- 一\n  - 嵌套\n\n1. 第一\n2. 第二\n\n> 引用\n\n---\n\n```ts\nconst value = "中文";\n```'} />);
    expect(screen.getByRole("heading", { name: "标题", level: 1 })).toBeVisible();
    expect(screen.getByRole("heading", { name: "二级", level: 2 })).toBeVisible();
    expect(container.querySelector("strong")).toHaveTextContent("粗体");
    expect(container.querySelector("em")).toHaveTextContent("斜体");
    expect(container.querySelector("ul ul")).toHaveTextContent("嵌套");
    expect(container.querySelector("ol")).toHaveTextContent("第二");
    expect(container.querySelector("blockquote")).toHaveTextContent("引用");
    expect(container.querySelector("hr")).not.toBeNull();
    expect(container.querySelector("p code")).toHaveTextContent("a < b");
    expect(container.querySelector("pre code")).toHaveTextContent('const value = "中文";');
  });
  it("renders GFM tables, strikethrough and inert task-list markers", () => {
    const { container } = render(<ChatMarkdown content={'~~旧内容~~\n\n- [x] 完成\n- [ ] 待办\n\n| 名称 | 数值 |\n| :--- | ---: |\n| 中文 | **42** |'} />);
    expect(container.querySelector("del")).toHaveTextContent("旧内容");
    expect(screen.getByRole("checkbox", { name: "已完成任务" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "未完成任务" })).toBeDisabled();
    const table = screen.getByRole("table");
    expect(within(table).getAllByRole("columnheader")).toHaveLength(2);
    expect(within(table).getByRole("cell", { name: "42" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Markdown 表格" })).toHaveAttribute("tabindex", "0");
  });
  it("keeps fenced script and HTML as exact inert text", () => {
    const code = '<img src="https://example.com/track" onerror="alert(1)">\n<script>fetch("https://example.com")</script>\n';
    const { container } = render(<ChatMarkdown content={`\`\`\`html\n${code}\`\`\``} />);
    expect(container.querySelector("pre code")?.textContent).toBe(code);
    expect(container.querySelector("img,script,iframe,style,link,object,embed,video,audio,svg")).toBeNull();
  });
  it("renders raw HTML as text without creating HTML, CSS, script or network-capable elements", () => {
    const fetch = vi.fn(), open = vi.fn();
    vi.stubGlobal("fetch", fetch); vi.stubGlobal("open", open);
    const { container } = render(<ChatMarkdown content={'<script>alert(1)</script>\n\n<iframe src="https://example.com"></iframe>\n\n<style>body { background: url(https://example.com/track) }</style>\n\n<img src="https://example.com/track" onerror="alert(1)">\n\n<svg onload="alert(1)"></svg>\n\n<link rel="stylesheet" href="https://example.com/style">\n\n<form action="https://example.com"><input autofocus></form>'} />);
    expect(container.querySelector("script,iframe,style,img,svg,link,form,input,object,embed")).toBeNull();
    expect(container.textContent).toContain("<script>alert(1)</script>");
    expect(fetch).not.toHaveBeenCalled(); expect(open).not.toHaveBeenCalled();
  });
  it.each(["https://example.com/pixel.png", "http://127.0.0.1/private", "data:image/svg+xml,test", "file:///C:/secret.png", "asset://localhost/secret.png", "//example.com/pixel.png"])("never instantiates or loads a Markdown image (%s)", (src) => {
    const { container } = render(<ChatMarkdown content={`![中文图片](${src})`} />);
    expect(screen.getByRole("note")).toHaveTextContent("[图片未加载：中文图片]");
    expect(container.querySelector("img,[src],[srcset]")).toBeNull();
    expect(container.textContent).not.toContain(src);
  });
  it.each(["javascript:alert%281%29", "JaVaScRiPt:alert%281%29", "jav&#x61;script:alert%281%29", "data:text/html,hello", "file:///C:/secret", "tauri://localhost/action", "asset://localhost/file", "mailto:person@example.com", "vbscript:hello", "//example.com/path", "/relative/path", "#local", "https://user:password@example.com/path", "https%3A%2F%2Fexample.com"])("rejects unsafe or non-HTTP(S) link destinations (%s)", (url) => {
    const { container } = render(<ChatMarkdown content={`[链接标签](${url})`} />);
    expect(screen.getByText("链接标签")).toBeVisible();
    expect(container.querySelector("a,[href],.markdown-link-address")).toBeNull();
  });
  it("keeps safe destinations readable without app navigation or window opening", async () => {
    const open = vi.fn(); vi.stubGlobal("open", open);
    const before = window.location.href;
    const { container } = render(<ChatMarkdown content="[官方网站](https://example.com/docs?q=中文#part)\n\nhttps://example.com/auto" />);
    expect(screen.getByText("（https://example.com/docs?q=%E4%B8%AD%E6%96%87#part）")).toBeVisible();
    await userEvent.click(screen.getByText("官方网站"));
    expect(window.location.href).toBe(before); expect(open).not.toHaveBeenCalled();
    expect(container.querySelector("a,[href],[target]")).toBeNull();
  });
  it("copies the original Markdown verbatim using the existing clipboard API", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined); vi.stubGlobal("navigator", { clipboard: { writeText } });
    const content = '# 标题\n\n[链接](javascript:alert%281%29)\n\n```ts\n\tconst 中文 = "<script>";\n```';
    render(<ChatMarkdown content={content} />);
    fireEvent.click(screen.getByRole("button", { name: "复制原文" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(content));
    expect(await screen.findByText("已复制")).toBeVisible();
  });
  it.each([
    ["```js\nfoo", "foo"], ["```\n```", ""], ["```\nfoo\n```", "foo"],
    ["```\nfoo\n\n\n```", "foo\n\n"], ["```\r\nfoo\r\nbar\r\n```", "foo\r\nbar"],
    ['```html\n<script>alert(1)</script>\n```', '<script>alert(1)</script>'],
  ])("copies code without the parser's synthetic final newline (%j)", async (source, code) => {
    const writeText = vi.fn().mockResolvedValue(undefined); vi.stubGlobal("navigator", { clipboard: { writeText } });
    render(<ChatMarkdown content={source} />);
    fireEvent.click(screen.getByRole("button", { name: "复制代码" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(code));
  });
  it("shows a recoverable clipboard failure without replacing the message", async () => {
    vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) } });
    render(<ChatMarkdown content="**保留内容**" />);
    fireEvent.click(screen.getByRole("button", { name: "复制原文" }));
    expect(await screen.findByText("无法复制，请手动选择文本复制")).toBeVisible();
    expect(screen.getByText("保留内容")).toBeVisible();
  });
  it("does not claim new streaming text was copied after an earlier copy resolves", async () => {
    const copied = deferred<void>(), writeText = vi.fn(() => copied.promise);
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const { rerender } = render(<ChatMarkdown content="**旧内容**" />);
    fireEvent.click(screen.getByRole("button", { name: "复制原文" }));
    rerender(<ChatMarkdown content="**旧内容** 新片段" />);
    await act(async () => copied.resolve());
    expect(writeText).toHaveBeenCalledWith("**旧内容**");
    expect(screen.queryByText("已复制")).not.toBeInTheDocument();
  });
  it("accepts every streaming prefix, including unfinished fences, tables and emphasis", () => {
    const content = '# 中文标题\n\n**粗体**\n\n| 列 | 值 |\n| --- | --- |\n| 一 | 二 |\n\n```js\nconst value = "<script>";\n```';
    const { rerender, container } = render(<ChatMarkdown content="" />);
    for (let end = 1; end <= content.length; end++) rerender(<ChatMarkdown content={content.slice(0, end)} />);
    expect(screen.getByRole("heading", { name: "中文标题" })).toBeVisible();
    expect(screen.getByRole("table")).toBeVisible();
    expect(container.querySelector("pre code")).toHaveTextContent('const value = "<script>";');
    expect(container.querySelector("script")).toBeNull();
  });
  it("retains code DOM, horizontal scroll and keyboard focus across streamed content and fence closure", () => {
    const prefix = '# 回复\n\n```js\nconst a = 1;';
    const { rerender, container } = render(<ChatMarkdown content={prefix} />);
    const pre = container.querySelector("pre")!, button = screen.getByRole("button", { name: "复制代码" });
    pre.scrollLeft = 130; button.focus();
    rerender(<ChatMarkdown content={`${prefix}\nconst b = 2;\n\`\`\``} />);
    expect(container.querySelector("pre")).toBe(pre); expect(pre.scrollLeft).toBe(130);
    expect(screen.getByRole("button", { name: "复制代码" })).toBe(button); expect(button).toHaveFocus();
  });
  it("isolates a real parser recursion failure, preserves raw copy and recovers on a later update", async () => {
    // Expected React error-boundary diagnostics are not product telemetry.
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const content = "> ".repeat(4096) + 'private-marker<img src="https://example.com/track" onerror="alert(1)">';
    const { rerender, container } = render(<ChatMarkdown content={content} />);
    expect(screen.getByText("此段内容无法排版，已保留原文")).toBeVisible();
    expect(container.querySelector(".markdown-plain")?.textContent).toBe(content);
    expect(container.querySelector("img,script,[src],[href]")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "复制原文" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(content));
    expect(error.mock.calls.flat().map(String).join(" ")).not.toContain("private-marker");
    rerender(<ChatMarkdown content={"## 已恢复\n\n**下一段**"} />);
    expect(screen.getByRole("heading", { name: "已恢复" })).toBeVisible();
    expect(container.querySelector("strong")).toHaveTextContent("下一段");
    expect(container.querySelector(".markdown-plain")).toBeNull();
  });

  it("keeps large tables in a stable keyboard-scrollable wrapper when rows stream in", () => {
    const row = `|${Array.from({ length: 32 }, (_, i) => `字段${i}`).join("|")}|`;
    const table = `${row}\n|${Array(32).fill("---").join("|")}|\n${Array(30).fill(row).join("\n")}`;
    const { rerender } = render(<ChatMarkdown content={table} />);
    const wrapper = screen.getByRole("region", { name: "Markdown 表格" });
    wrapper.scrollLeft = 800; wrapper.focus(); rerender(<ChatMarkdown content={`${table}\n${row}`} />);
    expect(screen.getByRole("region", { name: "Markdown 表格" })).toBe(wrapper);
    expect(wrapper).toHaveFocus(); expect(wrapper.scrollLeft).toBe(800);
    expect(within(wrapper).getAllByRole("row")).toHaveLength(32);
  });
});

describe("chat Markdown integration", () => {
  it("renders streamed replies while preserving raw user display, multi-turn requests and history", async () => {
    const first = deferred<ChatBatch>(), second = deferred<ChatBatch>();
    const next = vi.fn().mockImplementationOnce(() => first.promise).mockImplementationOnce(() => second.promise).mockResolvedValue(batch("第二轮", true));
    const { api, controller } = await mountChat({ chatNext: next });
    const question = "请解释 `x < y` 与 **Markdown**";
    send(question); await waitFor(() => expect(api.chatStart).toHaveBeenCalledTimes(1));
    const userMessage = screen.getByRole("article", { name: "你的消息" });
    expect(within(userMessage).getByText(question)).toBeVisible();
    expect(userMessage.querySelector(".chat-markdown,strong,code")).toBeNull();
    const prefix = "# 第一轮\n\n```ts\nconst x = 1;";
    await act(async () => first.resolve(batch(prefix)));
    expect(await screen.findByRole("heading", { name: "第一轮" })).toBeVisible();
    const answer = screen.getByRole("article", { name: "Nexa 回复" });
    expect(within(answer).getByLabelText("代码块")).toHaveTextContent("const x = 1;");
    await act(async () => second.resolve(batch("\n```\n\n**完成**", true)));
    await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle"));
    const original = `${prefix}\n\`\`\`\n\n**完成**`;
    expect(controller.getSnapshot().messages.map((message) => message.content)).toEqual([question, original]);
    send("继续"); await waitFor(() => expect(api.chatStart).toHaveBeenCalledTimes(2));
    expect(vi.mocked(api.chatStart).mock.calls[1][0].messages).toEqual([{ role: "user", content: question }, { role: "assistant", content: original }, { role: "user", content: "继续" }]);
    await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle"));
    expect(screen.getAllByRole("article", { name: "Nexa 回复" })).toHaveLength(2);
  });
  it("preserves incomplete output and cancellation semantics without retrying", async () => {
    const first = deferred<ChatBatch>(), stopped = deferred<ChatBatch>();
    const next = vi.fn().mockImplementationOnce(() => first.promise).mockImplementationOnce(() => stopped.promise);
    const { api, controller } = await mountChat({ chatNext: next });
    send("生成代码"); await waitFor(() => expect(api.chatStart).toHaveBeenCalledOnce());
    const unfinished = "## 未完成\n\n```js\nconst value =";
    await act(async () => first.resolve(batch(unfinished)));
    expect(await screen.findByRole("heading", { name: "未完成" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "停止生成" }));
    await waitFor(() => expect(api.chatCancel).toHaveBeenCalledOnce());
    await act(async () => stopped.resolve({ request_id: "request-1", events: [{ type: "cancelled" }], terminal: true }));
    await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle"));
    expect(screen.getByText("不完整")).toBeVisible();
    expect(controller.getSnapshot().messages.at(-1)?.content).toBe(unfinished);
    expect(screen.getByLabelText("代码块")).toHaveTextContent("const value =");
    expect(api.chatStart).toHaveBeenCalledOnce(); expect(screen.getByRole("button", { name: "发送" })).toBeDisabled();
  });
  it("keeps the app and multi-turn history usable after adversarial nesting", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const raw = "> ".repeat(4096) + "private-answer";
    const { api, controller } = await mountChat({ chatNext: vi.fn().mockResolvedValueOnce(batch(raw, true)).mockResolvedValue(batch("## 正常回复", true)) });
    send("测试嵌套");
    await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle"));
    expect(await screen.findByText("此段内容无法排版，已保留原文")).toBeVisible();
    expect(controller.getSnapshot().messages.at(-1)?.content).toBe(raw);
    send("继续");
    await waitFor(() => expect(api.chatStart).toHaveBeenCalledTimes(2));
    expect(vi.mocked(api.chatStart).mock.calls[1][0].messages[1].content).toBe(raw);
    expect(await screen.findByRole("heading", { name: "正常回复" })).toBeVisible();
    await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle"));
    expect(screen.getByRole("heading", { name: "聊天测试" })).toBeVisible();
  });

  it("keeps the reader's transcript scroll position instead of following stream chunks", async () => {
    const first = deferred<ChatBatch>(), second = deferred<ChatBatch>();
    const { controller } = await mountChat({ chatNext: vi.fn().mockImplementationOnce(() => first.promise).mockImplementationOnce(() => second.promise) });
    send("长回复"); await act(async () => first.resolve(batch("## 首段\n\n已输出")));
    expect(await screen.findByRole("heading", { name: "首段" })).toBeVisible();
    const region = screen.getByRole("region", { name: "当前会话" });
    Object.defineProperties(region, { scrollHeight: { value: 2000 }, clientHeight: { value: 400 } });
    region.scrollTop = 200; fireEvent.scroll(region);
    const scrollIntoView = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
    await act(async () => second.resolve(batch("\n\n**新片段**", true)));
    await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle"));
    expect(scrollIntoView).not.toHaveBeenCalled(); expect(region.scrollTop).toBe(200);
    expect(screen.getByText("新片段")).toBeVisible();
  });
});
