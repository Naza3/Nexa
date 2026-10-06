import { Component, memo, useState } from "react";
import type { ReactNode } from "react";
import Markdown from "react-markdown";
import type { Components, UrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";

function CopyText({ text, label }: { text: string; label: string }) {
  const [result, setResult] = useState<{ text: string; copied: boolean } | null>(null);
  const current = result?.text === text ? result : null;
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setResult({ text, copied: true });
    } catch {
      setResult({ text, copied: false });
    }
  };
  return <span className="markdown-copy">
    <button type="button" className="text-button" onClick={() => void copy()}>{label}</button>
    <span role="status">{current ? current.copied ? "已复制" : "无法复制，请手动选择文本复制" : ""}</span>
  </span>;
}

// Markdown is untrusted model/user text. Only absolute HTTP(S) destinations are
// retained as copyable text; nothing in a reply can navigate the app or fetch a
// resource. Keep this gate even though the renderers below emit no href/src.
const readableUrl: UrlTransform = (value, key) => {
  if (key !== "href" || !/^https?:\/\//i.test(value) || [...value].some((char) => char.charCodeAt(0) <= 32 || char.charCodeAt(0) === 127 || char === "\\")) return "";
  try {
    const url = new URL(value);
    return (url.protocol === "https:" || url.protocol === "http:") && url.hostname && !url.username && !url.password ? value : "";
  } catch {
    return "";
  }
};

// Defined once so streaming text updates reconcile existing code/table DOM,
// including keyboard focus and horizontal scroll, rather than remounting it.
const components: Components = {
  a: ({ children, href }) => <span className="markdown-link">
    {children}{href && <span className="markdown-link-address">（{href}）</span>}
  </span>,
  img: ({ alt }) => <span className="markdown-image" role="note">[图片未加载{alt ? `：${alt}` : ""}]</span>,
  pre: ({ children, node }) => {
    const code = node?.children.find((child) => child.type === "element" && child.tagName === "code");
    const renderedText = code?.type === "element" ? code.children.map((child) => child.type === "text" ? child.value : "").join("") : "";
    // mdast-util-to-hast adds one LF to nonempty code nodes. It is display
    // formatting, not source code; preserve all preceding blank lines/CRLF.
    const text = renderedText.endsWith("\n") ? renderedText.slice(0, -1) : renderedText;
    return <div className="markdown-code-block">
      <div className="markdown-code-tools"><CopyText text={text} label="复制代码" /></div>
      <pre tabIndex={0} aria-label="代码块">{children}</pre>
    </div>;
  },
  table: ({ children }) => <div className="markdown-table-scroll" tabIndex={0} role="region" aria-label="Markdown 表格"><table>{children}</table></div>,
  // Only the GFM parser may produce inputs; they are inert task-list markers.
  input: ({ checked }) => <input type="checkbox" checked={!!checked} disabled readOnly aria-label={checked ? "已完成任务" : "未完成任务"} />,
};
const remarkPlugins = [remarkGfm];
const allowedElements = [
  "p", "br", "hr", "h1", "h2", "h3", "h4", "h5", "h6", "strong", "em", "del",
  "blockquote", "ul", "ol", "li", "pre", "code", "a", "img", "table", "thead",
  "tbody", "tr", "th", "td", "input", "section", "sup",
];

type MarkdownBoundaryProps = { content: string; children: ReactNode };
type MarkdownBoundaryState = { content: string; failed: boolean };

// An adversarial but bounded reply can still exceed a parser's recursion limit.
// Isolate that failure to this reply; never log the message or lose its raw text.
class MarkdownBoundary extends Component<MarkdownBoundaryProps, MarkdownBoundaryState> {
  state: MarkdownBoundaryState = { content: this.props.content, failed: false };
  static getDerivedStateFromProps(props: MarkdownBoundaryProps, state: MarkdownBoundaryState) {
    return props.content !== state.content ? { content: props.content, failed: false } : null;
  }
  static getDerivedStateFromError() { return { failed: true }; }
  render() {
    return this.state.failed ? <div className="markdown-fallback">
      <p className="small-note" role="status">此段内容无法排版，已保留原文</p>
      <div className="markdown-plain">{this.props.content}</div>
    </div> : this.props.children;
  }
}

export const ChatMarkdown = memo(function ChatMarkdown({ content }: { content: string }) {
  return <>
    <div className="chat-markdown">
      <MarkdownBoundary content={content}>
        <Markdown remarkPlugins={remarkPlugins} components={components} allowedElements={allowedElements} urlTransform={readableUrl}>{content}</Markdown>
      </MarkdownBoundary>
    </div>
    <div className="message-tools"><CopyText text={content} label="复制原文" /></div>
  </>;
});
