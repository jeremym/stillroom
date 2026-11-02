import { useEffect, useState } from "react";
import { Zone } from "@overtone-foundry/app-kit";
import { Glyph } from "../brand";
import { api } from "../api";
import { ArtifactsPanel } from "./ArtifactsPanel";
import { shortModel, whereWord } from "../models";
import type { RecipeInfo, Run } from "../types";

/* An example result a catalog recipe ships: a saved run record shown through the ordinary viewers,
   so a reader can see what the recipe makes without a model or a run. Nothing here can be edited
   or saved; it is not a run on your files. */

/** The model(s) that made a saved run, from its resolutions, else from its steps; "programs only" when none. */
export function madeWith(run: Run): string {
  const ids = Object.values(run.resolutions ?? {}).map((r) => r.modelId);
  if (ids.length === 0) for (const s of run.steps ?? []) if (s.model && !ids.includes(s.model)) ids.push(s.model);
  if (ids.length === 0) return "programs only, no model";
  return [...new Set(ids)].map((id) => `${shortModel(id)} (${whereWord(id)})`).join(", ");
}

interface Props {
  recipe: RecipeInfo;
  onError: (message: string) => void;
}

export function ExampleButton({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  return <button onClick={onToggle} aria-expanded={open}><Glyph name="history" /> {open ? "Hide the example" : "See an example result"}</button>;
}

export function ExampleZone({ recipe, onError }: Props) {
  const [run, setRun] = useState<Run | null>(null);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let live = true;
    setLoading(true);
    setRun(null);
    api.exampleResult(recipe.id)
      .then((r) => { if (live) setRun(r); })
      .catch((e) => { if (live) onError(String(e)); })
      .finally(() => { if (live) setLoading(false); });
    return () => { live = false; };
  }, [recipe.id, onError]);

  const made = run ? `made ${new Date(run.startedAt).toLocaleDateString()} with ${madeWith(run)}` : loading ? "loading…" : "could not be loaded";
  return (
    <Zone title="An example result" sub={made} aside={<span className="tag">example</span>}>
      <p className="example-note">You are looking at a saved example, not a run on your files.</p>
      {run && <ArtifactsPanel run={run} outputs={recipe.outputs} steps={recipe.steps} busy={false} readOnly />}
      {run && Object.keys(run.outputs ?? {}).length === 0 && <p className="muted">The example recorded no results.</p>}
    </Zone>
  );
}
