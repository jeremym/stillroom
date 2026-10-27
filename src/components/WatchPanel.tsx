import { useState } from "react";
import { api } from "../api";
import type { WatchView } from "../types";

/* Folder watches: recipes that run by themselves when something lands in a folder. The app
   has to be open; nothing runs in the background and nothing listens on the network. */

interface Props {
  watches: WatchView[];
  onChanged: () => void;
  onOpenRun: (id: string) => void;
  onError: (message: string) => void;
}

const PHASE_WORD: Record<WatchView["phase"], string> = {
  watching: "watching",
  running: "running…",
  queued: "running… (more landed; runs again after)",
  paused: "paused",
  failed: "not watching",
};

function shortPath(p: string) {
  const home = p.match(/^\/Users\/[^/]+/)?.[0];
  return home ? p.replace(home, "~") : p;
}

export function WatchPanel({ watches, onChanged, onOpenRun, onError }: Props) {
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  if (watches.length === 0) return null;
  const act = (p: Promise<unknown>) => p.then(onChanged).catch((e) => onError(String(e)));

  return (
    <section className="panel">
      <h2>Folder watches <span className="muted">(recipes that run on their own while Stillroom is open)</span></h2>
      <ul className="watches">
        {watches.map((w) => (
          <li key={w.id}>
            <div>
              <strong>{w.recipeName}</strong>
              <span className="muted"> on </span>
              <span className="mono">{shortPath(w.folder)}</span>
              <span className={`tag ${w.phase === "running" || w.phase === "queued" ? "live" : w.phase === "failed" ? "bad" : ""}`}>{PHASE_WORD[w.phase]}</span>
            </div>
            <div className="muted">
              {w.lastFiredAt
                ? <>last ran {new Date(w.lastFiredAt).toLocaleString()}{w.lastRunId && <> · <button className="link" onClick={() => onOpenRun(w.lastRunId!)}>open that run</button></>}</>
                : "has not run yet"}
              {w.note && <> · <span className="mono">{w.note}</span></>}
            </div>
            <div className="row">
              {w.enabled
                ? <button onClick={() => act(api.setWatchEnabled(w.id, false))}>Pause</button>
                : <button onClick={() => act(api.setWatchEnabled(w.id, true))}>Resume</button>}
              <button disabled={!w.enabled || w.phase === "running" || w.phase === "queued"} onClick={() => act(api.fireWatch(w.id))}>Run now</button>
              {confirmRemove === w.id
                ? <button className="danger" onClick={() => { setConfirmRemove(null); act(api.removeWatch(w.id)); }}>Really remove</button>
                : <button onClick={() => setConfirmRemove(w.id)}>Remove…</button>}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
