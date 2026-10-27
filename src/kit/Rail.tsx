import type { ReactNode } from "react";

/* The Rail: a strip stuck to the top of the window. Left is where you are (a breadcrumb), right
   is what you can always do (status, new, settings, help). It does the job of a menu bar you can see. */
export function Rail({ left, right }: { left: ReactNode; right?: ReactNode }) {
  return (
    <header className="rail">
      <div className="rail-inner">
        <div className="rail-left">{left}</div>
        {right && <div className="rail-right">{right}</div>}
      </div>
    </header>
  );
}

/** A small status capsule: a dot, a word or two, optionally clickable. */
export function Pill({ ok, children, onClick, title }: { ok?: boolean; children: ReactNode; onClick?: () => void; title?: string }) {
  const body = <>{ok != null && <span className={`dot ${ok ? "ok" : "bad"}`} />}{children}</>;
  return onClick
    ? <button type="button" className="pill" onClick={onClick} title={title}>{body}</button>
    : <span className="pill" title={title}>{body}</span>;
}
