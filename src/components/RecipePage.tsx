import { useEffect, useState, type ReactNode } from "react";
import { api, fmtTokens } from "../api";
import { describePermissions } from "../permissions";
import { Glyph, inputGlyph, isGlyphName, outputGlyph, stepGlyph } from "../brand";
import { Empty, Zone } from "@overtone-foundry/app-kit";
import { WatchPanel } from "./WatchPanel";
import { keepHeldWord } from "../stages";
import { sourceWord } from "../catalog";
import { needsPermission } from "../consent";
import { ExampleButton, ExampleZone } from "./ExampleZone";
import type { InstalledModel, JournalEntry, KeepState, RecipeInfo, RunSummary, StepInfo, WatchView } from "../types";

/* The recipe's own page: what it does, what it needs, its runs, its watches, how it works. The same
   page serves the catalog's Look in a read-only form: Add to shelf stands where Set up a run would,
   and nothing that would change or run the recipe is offered until it is shelved. It also serves the
   import's preview (`preview`): a recipe not in the app yet, so no runs, no watches, no map, and the
   import's own buttons in place of the action row. */
interface Props {
  recipe: RecipeInfo;
  models: InstalledModel[];
  history: RunSummary[];
  watches: WatchView[];
  map: ReactNode;
  onSetup: () => void;
  onDoor: () => void;
  onOpenRun: (id: string) => void;
  onDuplicate: () => void;
  onRename: (name: string) => void;
  onDelete: () => void;
  /** Kept stages' state, by stage id. */
  keeps?: Record<string, KeepState>;
  onWatchesChanged: () => void;
  onError: (message: string) => void;
  /** The catalog's Look: the recipe is not on the shelf yet. */
  readOnly?: boolean;
  onAddToShelf?: () => void;
  /** For a duplicate: the catalog recipe it was made from, when the engine still has it. */
  origin?: RecipeInfo | null;
  /** Open a catalog recipe's page (Look). */
  onLook?: (id: string) => void;
  /** The import's preview: the recipe is not in the app yet. */
  preview?: boolean;
  /** Stands in place of the action row (the import's Add and Discard). */
  actions?: ReactNode;
}

const RUN_WORD: Record<string, string> = { succeeded: "done", failed: "failed", cancelled: "stopped", needs_review: "needs you", running: "running" };

export function RecipePage({ recipe, models, history, watches, map, onSetup, onDoor, onOpenRun, onDuplicate, onRename, onDelete, onWatchesChanged, onError, keeps, readOnly = false, onAddToShelf, origin, onLook, preview = false, actions }: Props) {
  const [exampleOpen, setExampleOpen] = useState(false);
  useEffect(() => setExampleOpen(false), [recipe.id]);
  const [journalFor, setJournalFor] = useState<string | null>(null);
  const [journal, setJournal] = useState<JournalEntry[]>([]);
  useEffect(() => {
    if (!journalFor) return;
    api.keepJournal(recipe.id, journalFor).then(setJournal).catch((e) => onError(String(e)));
  }, [journalFor, recipe.id, onError, keeps]);
  const [renaming, setRenaming] = useState(false);
  const [newName, setNewName] = useState(recipe.name);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const icon = isGlyphName(recipe.icon) ? recipe.icon : "recipe";
  const runs = history.filter((h) => h.recipeId === recipe.id);
  const mine = watches.filter((w) => w.recipeId === recipe.id);
  const yours = recipe.source === "user";
  const unfillable = Object.entries(recipe.modelSlots).filter(([, s]) => !models.some((m) => s.requires.every((c) => m.capabilities.includes(c as never))));
  const required = Object.entries(recipe.inputs).filter(([, i]) => i.required !== false);
  const optional = Object.entries(recipe.inputs).filter(([, i]) => i.required === false);
  const from = recipe.from ?? null;

  return (
    <>
      <Zone className="accent" title={<span className="recipe-title"><Glyph name={icon} className="lg" /> {recipe.name}</span>}
        aside={<>
          <span className="tag">{sourceWord(recipe.source)}</span>
          {recipe.source === "community" && recipe.author?.name && <span className="tag" title="Contributed through the repository and reviewed before it shipped">by {recipe.author.name}</span>}
          {preview ? <span className="tag amber">not added yet</span> : readOnly && <span className="tag">in the catalog</span>}
          {needsPermission(recipe) && !preview && <span className="tag amber" title="Set up asks you to tick what it may touch before it runs">needs your permission</span>}
          <span className="tag">v{recipe.version ?? "0"}</span><span className="tag">{recipe.steps.length} steps</span>
        </>}>
        {recipe.description && <p>{recipe.description}</p>}
        {from && (
          <p className="quiet-line provenance">
            <Glyph name="copy" /> Made from <b>{origin?.name ?? from.catalog}</b> {from.version}
            {from.newer && origin && <>; the catalog now has {origin.version ?? "a newer version"}. {onLook && <button className="link" onClick={() => onLook(origin.id)}>Look at it</button>}</>}
          </p>
        )}
        <div className="facts">
          <span className="label">Stages</span>
          <span className="stage-list">
            {recipe.stages.map((st, i) => (
              <span key={st.id} className="stage-row">
                <span className="stage-n">{i + 1}</span>
                <span><strong>{st.name}</strong>{st.why && <span className="muted"> · {st.why}</span>}
                  {st.keep && (() => {
                    const k = keeps?.[st.id];
                    const w = k ? keepHeldWord(k) : null;
                    return (
                      <span className="keep-line">
                        {w && <span className={`tag ${w.tone === "amber" ? "amber" : ""}`}>{w.status}</span>}
                        <span className="muted"> remembers between runs (a run keep)</span>
                        {k && k.fresh != null && (
                          <button className="link" onClick={() => setJournalFor(journalFor === st.id ? null : st.id)}>{journalFor === st.id ? "hide history" : "history"}</button>
                        )}
                        {journalFor === st.id && (
                          <ul className="journal">
                            {journal.length === 0 && <li className="muted">No runs yet.</li>}
                            {journal.map((j, n) => (
                              <li key={n}><span className="mono">{new Date(j.at).toLocaleString()}</span> · +{j.added} added · {j.replaced} replaced · {j.removed} removed · {j.reason} · <span className="mono">{j.runId.slice(0, 8)}</span></li>
                            ))}
                          </ul>
                        )}
                      </span>
                    );
                  })()}
                  <span className="strip">
                    {(st.steps ?? []).map((id) => recipe.steps.find((s) => s.id === id)).filter((s): s is StepInfo => Boolean(s)).map((s, k) => (
                      <span key={s.id} style={{ display: "contents" }}>
                        {k > 0 && <span className="arrow">›</span>}
                        <span className={`step ${s.kind === "model" || s.kind === "agent" ? "ai" : ""} ${s.kind === "review" ? "review" : ""}`}><Glyph name={stepGlyph(s.kind)} /> {s.name ?? s.id}</span>
                      </span>
                    ))}
                  </span>
                </span>
              </span>
            ))}
          </span>
          <span className="label">Needs</span>
          <span>
            {required.length === 0 ? <span className="muted">nothing</span> : required.map(([k, i]) => <span key={k} className="fact"><Glyph name={inputGlyph(i.type)} /> {i.label ?? k}</span>)}
            {optional.length > 0 && <span className="muted">· optional: {optional.map(([k, i]) => i.label ?? k).join(", ")}</span>}
          </span>
          <span className="label">Gives you</span>
          <span>{Object.entries(recipe.outputs).map(([k, o]) => <span key={k} className="fact"><Glyph name={outputGlyph(o.type)} /> {o.label ?? k}</span>)}</span>
          <span className="label">Thinks with</span>
          <span>
            {Object.keys(recipe.modelSlots).length === 0 ? <span className="muted">no model: programs only</span> :
              Object.entries(recipe.modelSlots).map(([k, s]) => (
                <span key={k} className="fact"><Glyph name="ai" /> {k} <span className="muted">· needs {s.requires.join(", ")}</span>{unfillable.some(([u]) => u === k) && <span className="tag amber">no model installed</span>}</span>
              ))}
          </span>
          <span className="label">May touch</span>
          <span className="muted">{describePermissions(recipe.permissions, recipe.inputs)}. Nothing else.</span>
        </div>
        {actions !== undefined ? actions : (
        <div className="row" style={{ marginTop: 18 }}>
          {readOnly ? (
            <>
              <button className="primary" style={{ marginTop: 0 }} onClick={onAddToShelf}><Glyph name="plus" /> Add to shelf</button>
              <span className="muted">Put it on your shelf to set up a run or make a copy of your own.</span>
            </>
          ) : (
            <>
              <button className="primary" style={{ marginTop: 0 }} onClick={onSetup}><Glyph name="power" /> Set up a run</button>
              <button onClick={onDoor}><Glyph name="door" /> Recipe card</button>
              <button onClick={onDuplicate}><Glyph name="copy" /> Duplicate</button>
            </>
          )}
          {yours && !renaming && <button onClick={() => { setNewName(recipe.name); setRenaming(true); }}><Glyph name="edit" /> Rename</button>}
          {yours && (confirmDelete
            ? <><button className="danger" onClick={() => { setConfirmDelete(false); onDelete(); }}>Really delete</button><button onClick={() => setConfirmDelete(false)}>Keep it</button></>
            : <button onClick={() => setConfirmDelete(true)}><Glyph name="delete" /> Delete…</button>)}
          {recipe.example && !preview && <ExampleButton open={exampleOpen} onToggle={() => setExampleOpen((o) => !o)} />}
        </div>
        )}
        {renaming && (
          <div className="row">
            <input type="text" value={newName} autoFocus onChange={(e) => setNewName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && newName.trim()) { onRename(newName.trim()); setRenaming(false); } if (e.key === "Escape") setRenaming(false); }} style={{ flex: 1, minWidth: 240 }} />
            <button className="primary" style={{ marginTop: 0 }} disabled={!newName.trim()} onClick={() => { onRename(newName.trim()); setRenaming(false); }}>Save name</button>
            <button onClick={() => setRenaming(false)}>Cancel</button>
          </div>
        )}
      </Zone>

      {exampleOpen && recipe.example && !preview && <ExampleZone recipe={recipe} onError={onError} />}

      {preview ? null : <>
      <Zone title="Runs of this recipe" sub="newest first" aside={<>{runs.length} run{runs.length === 1 ? "" : "s"}</>}>
        {runs.length === 0 ? (
          <Empty word="No runs yet"><p>{readOnly ? "Add it to the shelf, then set up a run; the results land here." : "Set up a run and the results land here."}</p></Empty>
        ) : (
          <ul className="rows runs">
            {runs.map((h) => (
              <li key={h.id}>
                <button className="link" onClick={() => onOpenRun(h.id)}>
                  {new Date(h.startedAt).toLocaleString()}
                  <span className={`tag ${h.status === "needs_review" ? "amber" : h.status === "failed" ? "bad" : ""}`}>{RUN_WORD[h.status] ?? h.status}</span>
                  {h.trigger && <span className="tag">by a watch</span>}
                  <span className="muted"> · {fmtTokens(h.promptTokens + h.completionTokens)} tokens</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </Zone>

      {mine.length > 0 && <WatchPanel watches={mine} onChanged={onWatchesChanged} onOpenRun={onOpenRun} onError={onError} title="Watches on this recipe" />}

      <Zone title="How it works" sub="the recipe as a map" aside={readOnly ? undefined : <button className="quiet" onClick={onDoor}>Open the recipe card to edit</button>}>
        {map}
      </Zone>
      </>}
    </>
  );
}
