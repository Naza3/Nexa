import { useEffect, useEffectEvent, useRef } from "react";
import type { ReactNode } from "react";

export function Modal({
  title,
  children,
  confirm,
  onConfirm,
  onCancel,
  danger = false,
  confirmDisabled = false,
}: {
  title: string;
  children: ReactNode;
  confirm: string;
  onConfirm: () => void;
  onCancel: () => void;
  danger?: boolean;
  confirmDisabled?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const cancel = useEffectEvent(onCancel);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const handle = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancel();
      }
      if (event.key === "Tab") {
        const items = ref.current?.querySelectorAll<HTMLButtonElement>(
          "button:not(:disabled)",
        );
        if (!items?.length) return;
        const first = items[0],
          last = items[items.length - 1];
        if (![...items].includes(document.activeElement as HTMLButtonElement)) {
          event.preventDefault();
          first.focus();
        } else if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener("keydown", handle);
    return () => {
      document.removeEventListener("keydown", handle);
      if (previous?.isConnected && !previous.matches(":disabled")) previous.focus();
      else {
        const heading = document.querySelector<HTMLElement>(".workspace-pages h1");
        if (heading) { heading.tabIndex = -1; heading.focus(); }
      }
    };
  }, []);
  return (
    <div className="modal-backdrop">
      <div
        ref={ref}
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="dialog-title"
      >
        <span className="eyebrow">请确认</span>
        <h2 id="dialog-title">{title}</h2>
        <div className="modal-copy">{children}</div>
        <div className="modal-actions">
          <button onClick={onCancel}>取消</button>
          <button
            className={danger ? "danger-button" : "primary"}
            disabled={confirmDisabled}
            onClick={onConfirm}
          >
            {confirm}
          </button>
        </div>
      </div>
    </div>
  );
}
