import { useEffect, useRef, useState } from "react";
import { fmtElapsed } from "@overtone-foundry/app-kit";
import { Glyph } from "../brand";
import { callWord, modelPhaseWord, phaseWord, type PlanStreamState } from "../planStream";

/* The planner's window: a dim, read-only block in the mono face under Draft it. A header says which
   call is in flight, how long the plan has run, and how many tokens the model has written; under it,
   the last dozen lines of what the model is writing, with a caret while it streams. The JSON is shown
   as it is: it is what is happening, and the stage names and whys inside it read fine. Only the header
   is announced; the text would be noise to a screen reader. The clock ticks here between events, so it
   moves even while the model is silent. */

export function planHeader(s: PlanStreamState, now: number): string | null {
  const word = phaseWord(s);
  if (!word) return null;
  const live = !s.done && s.at > 0 ? Math.max(0, now - s.at) : 0;
  const secs = Math.round((s.elapsedMs + live) / 1000);
  return [word, callWord(s), s.done ? null : modelPhaseWord(s.modelPhase), fmtElapsed(secs), `${s.tokens} tokens`].filter(Boolean).join(" · ");
}

export function PlanStreamView({ state }: { state: PlanStreamState }) {
  const [now, setNow] = useState(() => Date.now());
  const live = !state.done && state.phase != null;
  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [live]);
  // Each event brings its own clock; start counting from it.
  useEffect(() => { setNow(Date.now()); }, [state.at]);
  const ref = useRef<HTMLPreElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [state.text]);
  const header = planHeader(state, now);
  if (!header) return null;
  return (
    <div className="plan-stream">
      <div className="plan-stream-head" aria-live="polite">{header}</div>
      <pre className="plan-stream-text" ref={ref} aria-hidden="true">
        {state.lines.join("\n")}
        {live && <>{state.text && !state.text.endsWith("\n") ? " " : ""}<span className="cursor" /></>}
      </pre>
    </div>
  );
}

/* After a plan: the card's note with a quiet toggle beside it that reveals everything the model wrote,
   in the same dim block. Nothing the planner did is hidden. */
export function PlanNote({ note, transcript }: { note: string; transcript?: string | null }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="plan-note">
      <p className="result-line hint-line">
        <Glyph name="sparkles" /> {note}
        {transcript && (
          <> <button className="quiet" aria-expanded={open} onClick={() => setOpen((v) => !v)}>{open ? "hide what it wrote" : "show what it wrote"}</button></>
        )}
      </p>
      {open && transcript && (
        <div className="plan-stream">
          <div className="plan-stream-head">What the planner wrote, every call</div>
          <pre className="plan-stream-text tall">{transcript}</pre>
        </div>
      )}
    </div>
  );
}
