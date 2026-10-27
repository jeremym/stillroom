import { fmtTokens } from "../api";
import type { RunSummary } from "../types";

export function HistoryPanel({ history, onOpen }: { history: RunSummary[]; onOpen: (id: string) => void }) {
  if (history.length === 0) return null;
  return (
    <section className="panel">
      <h2>Previous runs</h2>
      <ul className="history">
        {history.map((h) => (
          <li key={h.id}>
            <button className="link" onClick={() => onOpen(h.id)}>
              {new Date(h.startedAt).toLocaleString()} · {h.recipeName} · {h.status === "needs_review" ? "needs your review" : h.status}{h.trigger ? " · by a watch" : ""} · {fmtTokens(h.promptTokens + h.completionTokens)} tokens
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
