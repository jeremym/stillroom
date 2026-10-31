import { useEffect, useMemo, useState } from "react";
import { Glyph, RECIPE_GLYPHS, type GlyphName } from "../brand";
import { Zone } from "@overtone-foundry/app-kit";
import { api } from "../api";
import { isLocalModel, shortModel, whereWord } from "../models";
import { slugify, uniqueId } from "../recipes/new";
import { PromptEditor } from "./PromptEditor";
import type { PlannerPreview, RecipeInfo } from "../types";

/* New recipe: a starting point, a name, an icon. Then the map opens in edit mode.
   From a goal: describe what the recipe should do (and paste what it should know); the planner,
   a model on this Mac unless you pick otherwise, drafts the recipe. The prompt it runs on is
   right here to read, change, and reset. */
export type NewRecipeMode = "blank" | "copy" | "goal";
export interface NewRecipeRequest {
  mode: NewRecipeMode;
  sourceId?: string;
  id: string;
  name: string;
  icon: GlyphName;
  goal?: string;
  material?: string;
  /** A model to plan with this time; null means Settings, then Auto. */
  model?: string | null;
}

interface Props {
  recipes: RecipeInfo[];
  busy: boolean;
  error: string | null;
  /** What the planner is doing right now, while busy. */
  progress?: string | null;
  /** What the checker still found after the planner's last try. */
  problems?: string[] | null;
  onCreate: (req: NewRecipeRequest) => void;
  onCancel: () => void;
  onStop?: () => void;
}

const VIA_WORD: Record<string, string> = { pinned: "your pick", default: "your default", recommended: "recommended", capability: "Auto" };

export function NewRecipe({ recipes, busy, error, progress, problems, onCreate, onCancel, onStop }: Props) {
  const [mode, setMode] = useState<NewRecipeMode>("goal");
  const [sourceId, setSourceId] = useState(recipes[0]?.id ?? "");
  const [name, setName] = useState("");
  const [icon, setIcon] = useState<GlyphName>("recipe");
  const [goal, setGoal] = useState("");
  const [material, setMaterial] = useState("");
  const [model, setModel] = useState<string>("");
  const [preview, setPreview] = useState<PlannerPreview | null>(null);
  const [showPrompt, setShowPrompt] = useState(false);
  const [promptError, setPromptError] = useState<string | null>(null);
  const taken = useMemo(() => recipes.map((r) => r.id), [recipes]);
  const id = uniqueId(slugify(name), taken);
  const ready = name.trim().length > 0 && (mode === "blank" || (mode === "copy" && Boolean(sourceId)) || (mode === "goal" && goal.trim().length > 0));

  useEffect(() => {
    if (mode !== "goal") return;
    let live = true;
    api.previewPlanner(model || null).then((p) => { if (live) setPreview(p); }).catch((e) => { if (live) setPromptError(String(e)); });
    return () => { live = false; };
  }, [mode, model]);

  const request = (): NewRecipeRequest => ({
    mode, sourceId, id, name: name.trim(), icon,
    ...(mode === "goal" ? { goal: goal.trim(), material: material.trim() || undefined, model: model || null } : {}),
  });
  const submit = () => { if (ready && !busy) onCreate(request()); };
  const picked = preview?.resolution ?? null;
  const planLabel = busy && mode === "goal" ? (progress ?? "Planning…") : busy ? "Creating…" : mode === "goal" ? "Draft it" : "Create and open its card";

  return (
    <Zone title="New recipe" sub="a reusable process you can run, watch, and share" className="accent">
      <h3>Start from</h3>
      <div className="choices">
        <button className="choice" aria-pressed={mode === "goal"} onClick={() => setMode("goal")}>
          <span><Glyph name="sparkles" /> <strong>A goal</strong></span>
          <span className="dim">Say what it should do. The planner drafts the steps; you check them on the map.</span>
        </button>
        <button className="choice" aria-pressed={mode === "blank"} onClick={() => setMode("blank")}>
          <span><Glyph name="file" /> <strong>Blank</strong></span>
          <span className="dim">Reads one file and hands it back. Add steps on the map.</span>
        </button>
        <button className="choice" aria-pressed={mode === "copy"} onClick={() => setMode("copy")}>
          <span><Glyph name="copy" /> <strong>A copy of a recipe</strong></span>
          <span className="dim">Everything from an existing recipe, under your name.</span>
        </button>
      </div>
      {mode === "copy" && (
        <div className="field">
          <label><strong>Copy of</strong></label>
          <select value={sourceId} onChange={(e) => setSourceId(e.target.value)}>
            {recipes.map((r) => <option key={r.id} value={r.id}>{r.name}{r.source === "builtin" ? " (built-in)" : ""}</option>)}
          </select>
        </div>
      )}

      {mode === "goal" && (
        <>
          <div className="field">
            <label><strong>What should this recipe do?</strong> <span className="muted">· what goes in, what you want out, who it is for</span></label>
            <textarea rows={5} value={goal} autoFocus placeholder="Read the web pages I give it about a topic and turn them into a study path: the key ideas in order, questions with answers, and the exact phrases I should be able to say."
              disabled={busy} onChange={(e) => setGoal(e.target.value)} />
          </div>
          <div className="field">
            <label><strong>Anything it should know</strong> <span className="muted">· optional: notes, links one per line, examples. They become the recipe's starting values.</span></label>
            <textarea rows={4} value={material} disabled={busy} placeholder="https://…&#10;https://…&#10;Notes about what matters most…" onChange={(e) => setMaterial(e.target.value)} />
          </div>
          <div className="field">
            <label><strong>Plan with</strong> <span className="muted">· the model that writes the draft. Auto picks the largest model on this Mac that answers in a fixed shape (JSON).</span></label>
            <div className="row" style={{ marginTop: 4 }}>
              <select value={model} disabled={busy} aria-label="Planning model" onChange={(e) => setModel(e.target.value)}>
                <option value="">Auto{picked && !model ? ` — ${shortModel(picked.modelId)} (${VIA_WORD[picked.via] ?? picked.via})` : ""}</option>
                {(preview?.candidates ?? []).map((m) => (
                  <option key={m.id} value={m.id}>{m.name}{m.parameterSize ? ` · ${m.parameterSize}` : ""} · {whereWord(m.id)}</option>
                ))}
              </select>
              {picked && <span className={isLocalModel(picked.modelId) ? "tag" : "tag amber"}>{whereWord(picked.modelId)}</span>}
              {preview?.problem && <span className="muted">{preview.problem}. Install a model in Settings › Models.</span>}
            </div>
          </div>
          <div className="field">
            <button className="quiet" aria-expanded={showPrompt} onClick={() => setShowPrompt((v) => !v)}>
              <Glyph name={showPrompt ? "collapse" : "expand"} /> {showPrompt ? "Hide the planner's prompt" : "Show the planner's prompt"}
            </button>
            {showPrompt && (
              <div style={{ marginTop: 6 }}>
                <p className="muted">This is what the planner is told, before your goal. The list of tools and the answer format are added at run time. Change it, save it as a revision, or reset to the original any time.</p>
                <PromptEditor name="planner" compact onError={setPromptError} />
                {promptError && <pre className="error">{promptError}</pre>}
              </div>
            )}
          </div>
        </>
      )}

      <div className="field">
        <label><strong>Name</strong> <span className="muted">· id will be <span className="mono">{id || "…"}</span></span></label>
        <input type="text" value={name} placeholder={mode === "goal" ? "Interview prep" : "Weekly report"} autoFocus={mode !== "goal"} disabled={busy}
          onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") submit(); }} />
      </div>

      <div className="field">
        <label><strong>Icon</strong> <span className="muted">· shown on the card and in the breadcrumb</span></label>
        <div className="icon-grid" role="radiogroup" aria-label="Icon">
          {RECIPE_GLYPHS.map((g) => (
            <button key={g} type="button" role="radio" aria-checked={icon === g} aria-pressed={icon === g} title={g} disabled={busy} onClick={() => setIcon(g)}><Glyph name={g} /></button>
          ))}
        </div>
      </div>

      {problems && problems.length > 0 && (
        <div className="field plan-problems">
          <p><Glyph name="warning" /> <strong>The planner's draft still has problems after three tries,</strong> so it was not saved. Try again (each try is a fresh roll), say more in the goal, or pick a larger model.</p>
          <ul>{problems.map((p, i) => <li key={i}>{p}</li>)}</ul>
        </div>
      )}
      {error && <pre className="error">{error}</pre>}
      <div className="row">
        <button className="primary" style={{ marginTop: 0 }} disabled={!ready || busy} onClick={submit}>
          {busy && mode === "goal" && <Glyph name="ai" />} {planLabel}
        </button>
        {busy && mode === "goal" && onStop ? <button className="stop" onClick={onStop}><Glyph name="stop" /> Stop</button> : <button disabled={busy} onClick={onCancel}>Cancel</button>}
      </div>
    </Zone>
  );
}
