import { useState } from "react";
import { Zone } from "@overtone-foundry/app-kit";
import { Glyph } from "../brand";
import { FindingsPanel } from "../flow/FindingsPanel";
import { problemsOf } from "../flow/findings";
import { allTicked, consentOf, fmtBytes, tickedGrants } from "../consent";
import { ConsentZone } from "./ConsentZone";
import { RecipePage } from "./RecipePage";
import type { Grant, ImportPreview, InstalledModel } from "../types";

/* Get a recipe…: the recipe the user picked (a folder or a zip), held in quarantine, shown as its
   Recipe page so it can be read before it joins your recipes. Under it, what the checker found, the
   consent step, its size, and the choice: Add to my recipes or Discard. Problems stop the import;
   so does an unticked line. Leaving this place without choosing discards. */

interface Props {
  preview: ImportPreview;
  models: InstalledModel[];
  busy: boolean;
  onConfirm: (grants: Grant[]) => void;
  onDiscard: () => void;
  onError: (message: string) => void;
}

const noop = () => {};

export function ImportPlace({ preview, models, busy, onConfirm, onDiscard, onError }: Props) {
  const [ticked, setTicked] = useState<Set<string>>(new Set());
  const { info, findings, sizeBytes } = preview;
  const consent = consentOf(info);
  const problems = problemsOf(findings);
  const ticksDone = allTicked(consent.needed, ticked);
  const ready = problems.length === 0 && ticksDone && !busy;
  const why = problems.length > 0
    ? `${problems.length} problem${problems.length === 1 ? "" : "s"} must be fixed in the recipe's folder first`
    : !ticksDone ? "tick every line above first" : null;

  return (
    <>
      <RecipePage recipe={info} models={models} history={[]} watches={[]} map={null} readOnly preview actions={null}
        onSetup={noop} onDoor={noop} onOpenRun={noop} onDuplicate={noop} onRename={noop} onDelete={noop} onWatchesChanged={noop} onError={onError} />

      <Zone title="Before it joins your recipes" sub="read what it may touch; nothing has been copied yet" className="accent"
        aside={<span>{fmtBytes(sizeBytes)}</span>}>
        <FindingsPanel findings={findings} blocks="Stops the import" />
        {consent.needed.length > 0
          ? <ConsentZone consent={consent} ticked={ticked} heading="What it may touch"
              note="Tick what you allow. Nothing is granted until you add the recipe."
              onTick={(key, on) => setTicked((t) => { const next = new Set(t); if (on) next.add(key); else next.delete(key); return next; })} />
          : <p className="muted">It touches only the files you give it and its own results. Nothing to allow.</p>}
        <p className="muted import-size">Size: {fmtBytes(sizeBytes)}{info.version ? ` · version ${info.version}` : ""} · goes into your recipes folder as <span className="mono">{info.id}</span></p>
        <div className="row">
          <button className="primary" style={{ marginTop: 0 }} disabled={!ready} onClick={() => onConfirm(tickedGrants(consent.needed, ticked))}>
            <Glyph name="plus" /> {busy ? "Adding…" : "Add to my recipes"}
          </button>
          <button disabled={busy} onClick={onDiscard}><Glyph name="delete" /> Discard</button>
          {why && !busy && <span className="muted">{why}.</span>}
        </div>
      </Zone>
    </>
  );
}
