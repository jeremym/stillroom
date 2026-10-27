import { useState, type ReactNode } from "react";
import { fmtTokens } from "../api";
import { describePermissions } from "../permissions";
import { Empty, Glyph, Zone, inputGlyph, isGlyphName, outputGlyph, stepGlyph } from "../kit";
import { WatchPanel } from "./WatchPanel";
import type { InstalledModel, RecipeInfo, RunSummary, WatchView } from "../types";

/* The recipe's own page: what it does, what it needs, its runs, its watches, how it works. */
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
  onWatchesChanged: () => void;
  onError: (message: string) => void;
}

const RUN_WORD: Record<string, string> = { succeeded: "done", failed: "failed", needs_review: "needs you", running: "running" };

export function RecipePage({ recipe, models, history, watches, map, onSetup, onDoor, onOpenRun, onDuplicate, onRename, onDelete, onWatchesChanged, onError }: Props) {
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

  return (
    <>
      <Zone className="accent" title={<span className="recipe-title"><Glyph name={icon} className="lg" /> {recipe.name}</span>}
        aside={<><span className="tag">{yours ? "yours" : "built-in"}</span><span className="tag">v{recipe.version ?? "0"}</span><span className="tag">{recipe.steps.length} steps</span></>}>
        {recipe.description && <p>{recipe.description}</p>}
        <div className="facts">
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
            <span className="tag">local</span>
          </span>
          <span className="label">May touch</span>
          <span className="muted">{describePermissions(recipe.permissions, recipe.inputs)}. Nothing else.</span>
          <span className="label">Steps</span>
          <span className="strip">
            {recipe.steps.map((s, i) => (
              <span key={s.id} style={{ display: "contents" }}>
                {i > 0 && <span className="arrow">›</span>}
                <span className={`step ${s.kind === "model" || s.kind === "agent" ? "ai" : ""} ${s.kind === "review" ? "review" : ""}`}><Glyph name={stepGlyph(s.kind)} /> {s.name ?? s.id}</span>
              </span>
            ))}
          </span>
        </div>
        <div className="row" style={{ marginTop: 18 }}>
          <button className="primary" style={{ marginTop: 0 }} onClick={onSetup}><Glyph name="power" /> Set up a run</button>
          <button onClick={onDoor}><Glyph name="door" /> Open the door</button>
          <button onClick={onDuplicate}><Glyph name="copy" /> Duplicate</button>
          {yours && !renaming && <button onClick={() => { setNewName(recipe.name); setRenaming(true); }}><Glyph name="edit" /> Rename</button>}
          {yours && (confirmDelete
            ? <><button className="danger" onClick={() => { setConfirmDelete(false); onDelete(); }}>Really delete</button><button onClick={() => setConfirmDelete(false)}>Keep it</button></>
            : <button onClick={() => setConfirmDelete(true)}><Glyph name="delete" /> Delete…</button>)}
        </div>
        {renaming && (
          <div className="row">
            <input type="text" value={newName} autoFocus onChange={(e) => setNewName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && newName.trim()) { onRename(newName.trim()); setRenaming(false); } if (e.key === "Escape") setRenaming(false); }} style={{ flex: 1, minWidth: 240 }} />
            <button className="primary" style={{ marginTop: 0 }} disabled={!newName.trim()} onClick={() => { onRename(newName.trim()); setRenaming(false); }}>Save name</button>
            <button onClick={() => setRenaming(false)}>Cancel</button>
          </div>
        )}
      </Zone>

      <Zone title="Runs of this recipe" sub="newest first" aside={<>{runs.length} run{runs.length === 1 ? "" : "s"}</>}>
        {runs.length === 0 ? (
          <Empty word="No runs yet"><p>Set up a run and the results land here.</p></Empty>
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

      <Zone title="How it works" sub="the recipe as a map" aside={<button className="quiet" onClick={onDoor}>Open the door to edit</button>}>
        {map}
      </Zone>
    </>
  );
}
