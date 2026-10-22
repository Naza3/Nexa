import { useRef, useState } from "react";
import { presentError } from "./errorPresentation";

/** A deliberately small support packet, never a serialized state or native error. */
export function ErrorDetails({ error }: { error: unknown }) {
  const [copying, setCopying] = useState(false);
  const [feedback, setFeedback] = useState<{ key: string; text: string } | null>(null);
  const pending = useRef(false);
  const safe = presentError(error);
  const packet = `Nexa 桌面诊断\n诊断码：${safe.code}\n说明：${safe.message}`;
  const copy = async () => {
    if (pending.current) return;
    pending.current = true; setCopying(true); setFeedback(null);
    try {
      await navigator.clipboard.writeText(packet);
      setFeedback({ key: packet, text: "诊断已复制，仅包含诊断码和受控说明。" });
    } catch {
      setFeedback({ key: packet, text: "复制失败，请手动复制上方诊断码。" });
    } finally { pending.current = false; setCopying(false); }
  };
  return <div className="small-note"><button type="button" disabled={copying} onClick={() => void copy()}>{copying ? "正在复制诊断…" : "复制诊断"}</button>{feedback?.key === packet && <p role="status">{feedback.text}</p>}</div>;
}
