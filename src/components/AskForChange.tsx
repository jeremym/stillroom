import { useState } from "react";
import { Zone } from "@overtone-foundry/app-kit";
import { api } from "../api";
import { Glyph } from "../brand";
import { problemTexts } from "../flow/findings";
import { shortModel, whereWord } from "../models";
import type { PlanStreamState } from "../planStream";
import type { Draft, EditOutcome, RecipeInfo } from "../types";
import { PlanNote, PlanStreamView } from "./PlanStreamView";

/* Ask for a change: tell the planner what is wrong with a recipe in words, read what it changed,
   then save the result as a copy (the original stays) or, for one of your own, in place. A built-in
   is never changed in place. Nothing is saved until a button is pressed. */

interface Props {
  recipe: RecipeInfo;
  /** The planner's window, fed by the app from `plan:progress`. */
  planStream: PlanStreamState;
  /** Ids already taken, so a copy gets a free one. */
  takenIds: string[];
  /** The app clears its planner window before a new plan starts. */
  onStart?: () => void;
  /** Something was saved; the app reloads its list and shows it. */
  onSaved: (recipe: RecipeInfo) => void;
}

/** `<id>-changed`, then `-changed-2`, `-changed-3`, … until one is free. */
export function changedId(id: string, taken: string[]): string {
  const base = `${id}-changed`;
  if (!taken.includes(base)) return base;
  let n = 2;
  while (taken.includes(`${base}-${n}`)) n += 1;
  return `${base}-${n}`;
}

/** Files whose text differs: (path, before, after). */
export function rewrittenFiles(before: Draft, after: Draft): [string, string, string][] {
  return Object.entries(after.files).filter(([p, t]) => p in before.files && before.files[p] !== t).map(([p, t]) => [p, before.files[p], t]);
}

export function AskForChange({ recipe, planStream, takenIds, onStart, onSaved }: Props) {
  const [instruction, setInstruction] = useState("");
  const [running, setRunning] = useState(false);
  const [before, setBefore] = useState<Draft | null>(null);
  const [out, setOut] = useState<EditOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const own = recipe.source === "user";

  async function plan() {
    if (!instruction.trim() || running) return;
    setRunning(true);
    setError(null);
    setOut(null);
    onStart?.();
    try {
      const src = await api.readRecipeDraft(recipe.id);
      setBefore(src.draft);
      setOut(await api.editRecipe({ recipeId: recipe.id, instruction }));
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  async function save(inPlace: boolean) {
    if (!out) return;
    setSaving(true);
    setError(null);
    try {
      const draft = inPlace
        ? out.draft
        : { yaml: { ...out.draft.yaml, id: changedId(recipe.id, takenIds), name: `${recipe.name} (changed)` }, files: out.draft.files };
      const r = await api.saveRecipeDraft(draft, !inPlace);
      setOut(null);
      setInstruction("");
      onSaved(r.recipe);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  const problems = out ? problemTexts(out.problems) : [];
  const clean = out != null && problems.length === 0;
  return (
    <Zone id="ask-change" title="Ask for a change" sub="edit by asking" className="ask-change">
      <p className="muted">Say what is wrong or what you want added, in your own words. The planner rewrites the recipe with only that change, and you read what moved before anything is saved.</p>
      <div className="field">
        <label htmlFor="ask-change-text"><strong>What should change?</strong></label>
        <textarea id="ask-change-text" rows={3} value={instruction} disabled={running} onChange={(e) => setInstruction(e.target.value)}
          placeholder="The quiz should come from the brief, not from pasted notes. Drop the voice script." />
      </div>
      <div className="row">
        {running
          ? <button className="stop" onClick={() => api.stopRun("plan").catch(() => {})}><Glyph name="stop" /> Stop</button>
          : <button className="primary" disabled={!instruction.trim() || saving} onClick={plan}><Glyph name="ai" /> Plan the change</button>}
        {out && !running && <span className="muted">Changed by {shortModel(out.modelId)} ({whereWord(out.modelId)}) in {Math.round(out.durationMs / 1000)} s.</span>}
      </div>
      {running && <PlanStreamView state={planStream} />}
      {error && <p className="error">{error}</p>}
      {out && !running && problems.length > 0 && (
        <>
          <p className="notice"><Glyph name="warning" /> The planner's change had problems the checker could not clear, so nothing was saved. Try saying it another way.</p>
          <ul className="plan-problems-list">{problems.map((p) => <li key={p}>{p}</li>)}</ul>
          <PlanNote note="Here is what it wrote." transcript={out.transcript} />
        </>
      )}
      {clean && out && (
        <div className="change-result">
          <h3>What changed <span className="muted">({out.changes.length === 0 ? "nothing" : `${out.changes.length} ${out.changes.length === 1 ? "thing" : "things"}`})</span></h3>
          {out.changes.length === 0
            ? <p className="muted">The planner returned the recipe as it was. Say it another way, or ask for something more.</p>
            : <ul className="change-list">{out.changes.map((c) => <li key={c}>{c}</li>)}</ul>}
          {before && rewrittenFiles(before, out.draft).map(([path, was, now]) => (
            <details key={path} className="change-file">
              <summary>{path} · before and after</summary>
              <div className="change-side-by-side">
                <pre className="change-before">{was}</pre>
                <pre className="change-after">{now}</pre>
              </div>
            </details>
          ))}
          <div className="row">
            <button className="primary" disabled={saving || out.changes.length === 0} onClick={() => save(false)}>Save as a copy</button>
            {own && <button disabled={saving || out.changes.length === 0} onClick={() => save(true)}>Save in place</button>}
            <button className="quiet" disabled={saving} onClick={() => setOut(null)}>Discard</button>
            <span className="muted">{own ? "A copy keeps this recipe as it is; in place replaces it." : "A built-in is never changed in place; the copy is yours."}</span>
          </div>
          <PlanNote note="Nothing is saved until you choose." transcript={out.transcript} />
        </div>
      )}
    </Zone>
  );
}
