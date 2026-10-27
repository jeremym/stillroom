import { useEffect, useState } from "react";
import { dig, asText } from "../viewers/dig";
import type { Run, StepInfo } from "../types";

/* The human checkpoint. A run that stopped at a review step shows its items here; approving
   continues the run with the approved ones (the decision is fed back as that step's output). */

interface Props {
  run: Run;
  steps: StepInfo[];
  busy: boolean;
  onDecide: (stepId: string, decision: { decided: true; approved: unknown[]; rejected: unknown[] }) => void;
}

function findStep(steps: StepInfo[], id: string): StepInfo | undefined {
  for (const s of steps) {
    if (s.id === id) return s;
    const inner = s.steps && findStep(s.steps, id);
    if (inner) return inner;
  }
  return undefined;
}

export function ReviewPanel({ run, steps, busy, onDecide }: Props) {
  const pending = run.steps.find((s) => s.kind === "review" && s.status === "skipped" && s.skipReason);
  const items = ((pending?.output as { items?: unknown[] } | undefined)?.items ?? []) as unknown[];
  const [checked, setChecked] = useState<boolean[]>([]);
  useEffect(() => setChecked(items.map(() => true)), [run.id, items.length]);
  if (!pending) return null;
  const info = findStep(steps, pending.stepId);
  const show = info?.show?.length ? info.show : [];
  const title = info?.title ?? pending.name ?? "Review";
  const approvedCount = checked.filter(Boolean).length;

  function decide(all: boolean | null) {
    const approved: unknown[] = [];
    const rejected: unknown[] = [];
    items.forEach((it, i) => ((all ?? checked[i]) ? approved : rejected).push(it));
    onDecide(pending!.stepId, { decided: true, approved, rejected });
  }

  return (
    <section className="panel review">
      <h2>{title} <span className="muted">(review step · the run is paused here)</span></h2>
      {items.length === 0 ? (
        <p className="muted">Nothing to review. Continue to finish the run.</p>
      ) : (
        <p className="muted">Untick anything you do not want. Nothing is written until you approve.</p>
      )}
      <ul className="review-items">
        {items.map((it, i) => {
          const headline = show[0] ? asText(dig(it, show[0])) : `Item ${i + 1}`;
          const rest = show.slice(1);
          return (
            <li key={i} className={checked[i] ? "" : "rejected"}>
              <label className="row" style={{ alignItems: "flex-start" }}>
                <input type="checkbox" checked={checked[i] ?? true}
                  onChange={(e) => setChecked((c) => c.map((x, k) => (k === i ? e.target.checked : x)))} />
                <div style={{ flex: 1 }}>
                  <strong>{headline || `Item ${i + 1}`}</strong>
                  {rest.map((p) => {
                    const v = asText(dig(it, p));
                    if (!v) return null;
                    return v.length > 120 || v.includes("\n")
                      ? <pre key={p} className="artifact review-body">{v}</pre>
                      : <div key={p} className="muted">{v}</div>;
                  })}
                  {show.length === 0 && <pre>{JSON.stringify(it, null, 2)}</pre>}
                </div>
              </label>
            </li>
          );
        })}
      </ul>
      <div className="row">
        <button className="primary" style={{ marginTop: 0 }} disabled={busy} onClick={() => decide(null)}>
          {busy ? "Continuing…" : items.length === 0 ? "Continue" : `Approve ${approvedCount} of ${items.length} and continue`}
        </button>
        {items.length > 0 && <button disabled={busy} onClick={() => decide(false)}>Reject all and continue</button>}
      </div>
    </section>
  );
}
