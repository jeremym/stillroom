import { useEffect, useRef, type KeyboardEvent } from "react";

/* The Dial: five place names in a row, a lens that slides to the one you are in, and the names
   beside it tilting away as if on a drum. Every place stays visible and clickable at all times.
   It is a real tablist underneath; the lens and the tilt are decoration. Reduced motion cuts instead of sliding. */

export interface DialPlace {
  id: string;
  label: string;
  /** Shown when the place has nothing yet; the place still opens and explains itself. */
  empty?: boolean;
}

export type LensTone = "green" | "amber";

interface Props {
  places: DialPlace[];
  current: string;
  onChange: (id: string) => void;
  tone?: LensTone;
  /** 0 to 1 while something is in flight; undefined otherwise. */
  progress?: number;
  /** One line under the lens: where you are, in plain words. */
  crumb?: React.ReactNode;
  itemWidth?: number;
}

export function nextPlace(places: DialPlace[], current: string, delta: number): string {
  const i = Math.max(0, places.findIndex((p) => p.id === current));
  const j = Math.min(places.length - 1, Math.max(0, i + delta));
  return places[j].id;
}

export function Dial({ places, current, onChange, tone = "green", progress, crumb, itemWidth = 196 }: Props) {
  const idx = Math.max(0, places.findIndex((p) => p.id === current));
  const listRef = useRef<HTMLDivElement>(null);

  // Keep focus on the selected tab when the dial turns by itself.
  useEffect(() => {
    const el = listRef.current;
    if (!el || !el.contains(document.activeElement)) return;
    (el.querySelector<HTMLButtonElement>(`[data-place="${current}"]`))?.focus();
  }, [current]);

  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowLeft") { e.preventDefault(); onChange(nextPlace(places, current, -1)); }
    if (e.key === "ArrowRight") { e.preventDefault(); onChange(nextPlace(places, current, 1)); }
    if (e.key === "Home") { e.preventDefault(); onChange(places[0].id); }
    if (e.key === "End") { e.preventDefault(); onChange(places[places.length - 1].id); }
  }

  const lensX = (idx - (places.length - 1) / 2) * itemWidth;
  return (
    <div className="dial-wrap">
      <div className="dial" role="tablist" aria-label="Places" ref={listRef} onKeyDown={onKey} style={{ ["--dial-item" as string]: `${itemWidth}px` }}>
        <div className="dial-drum">
          {places.map((p, i) => {
            const d = i - idx;
            const cls = d === 0 ? "here" : `${Math.abs(d) === 1 ? "near" : ""} ${d < 0 ? "before" : "after"}`;
            return (
              <button
                key={p.id} role="tab" data-place={p.id} aria-selected={d === 0} tabIndex={d === 0 ? 0 : -1}
                className={`dial-item ${cls} ${p.empty ? "empty-place" : ""}`}
                onClick={() => onChange(p.id)}
              >
                {p.label}
              </button>
            );
          })}
        </div>
        <div className={`dial-lens ${tone} ${progress != null ? "running" : ""}`} aria-hidden="true" style={{ transform: `translateX(calc(${lensX}px - 50%))` }}>
          {progress != null && <div className="dial-progress"><i style={{ width: `${Math.round(Math.min(1, Math.max(0, progress)) * 100)}%` }} /></div>}
        </div>
      </div>
      <p className="dial-crumb" aria-live="polite">{crumb}</p>
    </div>
  );
}
