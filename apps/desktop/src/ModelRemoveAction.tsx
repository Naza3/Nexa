import { useEffect, useId, useRef, useState } from "react";

/** Secondary row actions stay out of the compact model list's primary path. */
export function ModelRemoveAction({ name, reason, onRemove }: {
  name: string;
  reason: string | null;
  onRemove: () => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const id = useId();
  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      if (event.target instanceof Node && !ref.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);
  return <div ref={ref} className="model-more-actions" onBlur={(event) => {
    if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
  }} onKeyDown={(event) => {
    if (event.key === "Escape" && open) {
      event.preventDefault(); event.stopPropagation(); setOpen(false); trigger.current?.focus();
    }
  }}>
    <button ref={trigger} className="icon-button" aria-label={`${name} 的更多操作`} aria-expanded={open} aria-controls={id} onClick={() => setOpen(!open)}><span aria-hidden="true">···</span></button>
    {open && <div id={id} className="model-more-panel">
      <button disabled={!!reason} aria-describedby={reason ? `${id}-reason` : undefined} onClick={() => {
        setOpen(false); trigger.current?.focus(); onRemove();
      }}>从模型库移除</button>
      {reason && <p id={`${id}-reason`} className="small-note">{reason}</p>}
    </div>}
  </div>;
}
