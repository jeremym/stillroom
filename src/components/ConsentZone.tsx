import type { ReactNode } from "react";
import { Glyph } from "../brand";
import { allTicked, allowedWord, grantKey } from "../consent";
import type { Consent } from "../types";
import { Group } from "@overtone-foundry/app-kit";

/* The consent step: what a recipe may touch beyond its inputs and its own results, one line and one
   tick per entry. Set up shows it above the inputs and will not run the recipe until every line is
   ticked; the import (Get a recipe…) shows the same lines before anything is copied. Lines already
   allowed show ticked and set, with Take back to undo them all. */

interface Props {
  consent: Consent;
  /** Keys (see `grantKey`) of the needed lines ticked so far. */
  ticked: ReadonlySet<string>;
  onTick: (key: string, on: boolean) => void;
  /** Set up: take back every allowed line. Left out where nothing has been allowed yet (the import). */
  onRevoke?: () => void;
  /** While the engine is writing the answer. */
  busy?: boolean;
  /** The heading above the lines; `null` leaves it out when the zone around it has its own. */
  heading?: ReactNode | null;
  /** What to say under the lines when some are unticked. */
  note?: string;
}

export function ConsentZone({ consent, ticked, onTick, onRevoke, busy = false, heading = "What this recipe may touch", note = "Tick what you allow. The recipe will not run until every line is ticked." }: Props) {
  const { needed, granted } = consent;
  if (needed.length === 0 && granted.length === 0) return null;
  const done = allTicked(needed, ticked);
  const body = (
    <>
      <ul className="consent-lines">
        {needed.map((g) => {
          const key = grantKey(g);
          return (
            <li key={key}>
              <label className="consent-line">
                <input type="checkbox" checked={ticked.has(key)} disabled={busy} onChange={(e) => onTick(key, e.target.checked)} />
                <span>{g.text}</span>
              </label>
            </li>
          );
        })}
        {granted.map((g) => (
          <li key={`granted:${grantKey(g)}`}>
            <label className="consent-line allowed">
              <input type="checkbox" checked disabled readOnly />
              <span>{g.text} <span className="tag">{allowedWord(g)}</span></span>
            </label>
          </li>
        ))}
      </ul>
      {needed.length > 0 && !done && <p className="muted consent-note"><Glyph name="warning" /> {note}</p>}
      {needed.length > 0 && done && busy && <p className="muted consent-note">Allowing…</p>}
      {granted.length > 0 && onRevoke && (
        <p className="muted consent-note">
          <button className="quiet" disabled={busy} onClick={onRevoke}>Take back</button>
          <span>Takes back every line above; the recipe will not run until ticked again.</span>
        </p>
      )}
    </>
  );
  // Inside a zone with its own heading (the import), the lines stand alone; on Set up they are a group
  // with the amber rim until every line is ticked.
  if (heading === null) return <div className="consent-zone" role="group" aria-label="What this recipe may touch">{body}</div>;
  return <Group className="consent-zone" title={heading} sub="permissions" label="What this recipe may touch" needsYou={needed.length > 0 && !done}>{body}</Group>;
}
