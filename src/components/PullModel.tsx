import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { pullBar } from "../settings/pull";
import type { PullEvent } from "../types";

/* Pull a model through Ollama with a block-character progress bar. Used by Settings and the welcome. */
export function PullModel({ enabled, suggestion = "qwen3:8b", onDone }: { enabled: boolean; suggestion?: string; onDone?: () => void }) {
  const [name, setName] = useState("");
  const [pull, setPull] = useState<PullEvent | null>(null);
  const [pulling, setPulling] = useState(false);
  useEffect(() => {
    const un = listen<PullEvent>("model:pull", (ev) => setPull(ev.payload));
    return () => { un.then((f) => f()); };
  }, []);
  async function start(n = name) {
    const target = n.trim();
    if (!target) return;
    setPulling(true);
    setPull({ name: target, status: "starting", total: null, completed: null, done: false, error: null });
    try {
      await api.pullModel(target);
      onDone?.();
    } catch (e) {
      setPull({ name: target, status: "error", total: null, completed: null, done: true, error: String(e) });
    } finally {
      setPulling(false);
    }
  }
  const bar = pull ? pullBar(pull.completed, pull.total) : null;
  return (
    <>
      <div className="row field-row">
        <div className="field">
          <label><strong>Model</strong> <span className="muted">· a name from ollama.com/library, like <span className="mono">{suggestion}</span></span></label>
          <input type="text" className="mono" value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") start(); }} placeholder={suggestion} disabled={pulling} />
        </div>
        <button className="primary" style={{ marginTop: 0 }} onClick={() => start(name || suggestion)} disabled={pulling || !enabled}>{pulling ? "Pulling…" : name.trim() ? "Pull" : `Pull ${suggestion}`}</button>
      </div>
      {pull && bar && (
        <div className="pull">
          <div className="progress glow">{bar.bar} <span className="muted">{bar.pct != null ? `${bar.pct}%` : ""}</span></div>
          <p className={pull.error ? "red" : "muted"}>
            <span className="mono">{pull.name}</span> · {pull.error ?? pull.status}{bar.text ? ` · ${bar.text}` : ""}{pull.done && !pull.error ? " · ready to use" : ""}
          </p>
        </div>
      )}
    </>
  );
}
