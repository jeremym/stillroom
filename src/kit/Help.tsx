import { useEffect, useId, useRef, useState, type ReactNode } from "react";

/* The one way a short explanation pops up anywhere in the app: a small "?" that opens a card
   beside it. Click outside or press Escape to close. Nothing else in the app is a popup. */
export function Help({ title, children, label = "What is this?" }: { title?: ReactNode; children: ReactNode; label?: string }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);
  const id = useId();
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false); };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => { document.removeEventListener("mousedown", onDown); document.removeEventListener("keydown", onKey); };
  }, [open]);
  return (
    <span className="help" ref={ref}>
      <button type="button" className="help-btn" aria-label={label} aria-expanded={open} aria-controls={id} onClick={() => setOpen((o) => !o)}>?</button>
      {open && (
        <span className="help-card" role="dialog" id={id}>
          {title && <strong className="help-title">{title}</strong>}
          <span className="help-body">{children}</span>
        </span>
      )}
    </span>
  );
}

/** Seconds elapsed since `since`, ticking once a second while mounted. */
export function useElapsed(since: string | null | undefined, live = true): number | null {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!since || !live) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [since, live]);
  if (!since) return null;
  return Math.max(0, Math.round((now - new Date(since).getTime()) / 1000));
}

export function fmtElapsed(s: number): string {
  return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`;
}
