import type { ReactNode } from "react";

/* A zone: one section of a place, on its own ground. Consecutive zones alternate ground by CSS. */
export function Zone({ title, sub, aside, className = "", children }: {
  title?: ReactNode; sub?: ReactNode; aside?: ReactNode; className?: string; children: ReactNode;
}) {
  return (
    <section className={`zone ${className}`}>
      {(title || aside) && (
        <div className="zone-head">
          {title && <h2>{title} {sub && <span className="muted">({sub})</span>}</h2>}
          {aside && <span className="zone-aside">{aside}</span>}
        </div>
      )}
      {children}
    </section>
  );
}

export function Empty({ word, children }: { word: string; children?: ReactNode }) {
  return (
    <div className="empty">
      <span className="disp">{word}</span>
      {children}
    </div>
  );
}
