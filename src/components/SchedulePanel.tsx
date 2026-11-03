import { useState } from "react";
import { Toggle, Zone } from "@overtone-foundry/app-kit";
import { api } from "../api";
import { nextWord, whenWord } from "../schedules";
import type { Schedule } from "../types";

/* Schedules: recipes that run by themselves at a time of day, on the days chosen. The app has to be
   open; nothing runs in the background and nothing listens on the network. Running while the app is
   closed is roadmap (a menu-bar mode), not here. */

interface Props {
  schedules: Schedule[];
  /** How many folder watches the person has. With none and no schedules the panel hides; with a watch
   *  and no schedules it shows one line, so the person learns schedules exist. */
  watchCount?: number;
  onChanged: () => void;
  onOpenRun: (id: string) => void;
  onError: (message: string) => void;
  title?: string;
  /** For the tests: the clock the "next …" words are read against. */
  now?: () => Date;
}

export function SchedulePanel({ schedules, watchCount = 0, onChanged, onOpenRun, onError, title = "Schedules", now }: Props) {
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [firing, setFiring] = useState<Set<string>>(new Set());
  if (schedules.length === 0 && watchCount === 0) return null;
  const act = (p: Promise<unknown>) => p.then(onChanged).catch((e) => onError(String(e)));

  function runNow(id: string) {
    setFiring((s) => new Set(s).add(id));
    api.fireSchedule(id)
      .then(onChanged)
      .catch((e) => onError(String(e)))
      .finally(() => setFiring((s) => { const n = new Set(s); n.delete(id); return n; }));
  }

  return (
    <Zone title={title} sub="run a recipe at a time of day, while Stillroom is open" className="schedules-zone">
      {schedules.length === 0 ? (
        <p className="muted">No schedules yet. In a recipe's Set up, press <strong>Run every day at…</strong> to run it at the same time each day.</p>
      ) : (
        <ul className="watches schedules">
          {schedules.map((s) => {
            const next = s.enabled ? nextWord(s.nextAt, s.time, now?.()) : null;
            return (
              <li key={s.id}>
                <div className="row space" style={{ margin: 0 }}>
                  <div>
                    <strong>{s.recipeName}</strong>
                    <span className="muted"> {whenWord(s.time, s.days)}</span>
                    {next && <span className="tag">{next}</span>}
                    {!s.enabled && <span className="tag">off</span>}
                  </div>
                  <Toggle on={s.enabled} label={`${s.recipeName} ${whenWord(s.time, s.days)}`} onChange={(on) => act(api.setScheduleEnabled(s.id, on))} />
                </div>
                <div className="muted">
                  {s.lastFiredAt
                    ? <>last ran {new Date(s.lastFiredAt).toLocaleString()}{s.lastRunId && <> · <button className="link" onClick={() => onOpenRun(s.lastRunId!)}>open that run</button></>}</>
                    : "has not run yet"}
                </div>
                <div className="row">
                  <button disabled={firing.has(s.id)} onClick={() => runNow(s.id)}>{firing.has(s.id) ? "Running…" : "Run now"}</button>
                  {confirmDelete === s.id
                    ? <><button className="danger" onClick={() => { setConfirmDelete(null); act(api.deleteSchedule(s.id)); }}>Really delete</button><button onClick={() => setConfirmDelete(null)}>Keep it</button></>
                    : <button onClick={() => setConfirmDelete(s.id)}>Delete…</button>}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </Zone>
  );
}
