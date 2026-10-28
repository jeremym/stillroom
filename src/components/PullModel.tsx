import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { pullBar } from "../settings/pull";
import type { PullEvent } from "../types";
import { isInstalled, suggestPull } from "../models";

/* Pull a model through Ollama with a block-character progress bar. Used by Settings and the welcome. */
/* `installed` keeps the suggestion to a model you do not have yet, and stops a pull of one you do. */
export function PullModel({ enabled, installed = [], onDone }: { enabled: boolean; installed?: string[]; onDone?: () => void }) {
  const [name, setName] = useState("");
  const suggestion = suggestPull(installed);
  const already = isInstalled(name, installed);
  // With nothing installed the suggestion is the button; with models already here, pulling is a choice, not a prompt.
  const first = installed.length === 0;
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
          <label><strong>Model</strong> <span className="muted">{first ? <>· a name from ollama.com/library, like <span className="mono">{suggestion}</span></> : <>· only if you want another one · a name from ollama.com/library</>}</span></label>
          <input type="text" className="mono" value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") start(); }} placeholder={first ? suggestion : "model name"} disabled={pulling} />
        </div>
        <button className={first ? "primary" : ""} style={{ marginTop: 0 }} onClick={() => start(name || (first ? suggestion : ""))} disabled={pulling || !enabled || already || (!first && !name.trim())}>{pulling ? "Pulling…" : already ? "Already installed" : name.trim() || !first ? "Pull" : `Pull ${suggestion}`}</button>
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
