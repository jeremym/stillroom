import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Glyph, RECIPE_GLYPHS, type GlyphName } from "../brand";
import { Tabs, Tag, Zone, fmtElapsed, useElapsed, type TabItem } from "@overtone-foundry/app-kit";
import { api } from "../api";
import { isLocalModel, shortModel, whereWord } from "../models";
import { costLine, gb, optionLabel } from "../planWith";
import { slugify, uniqueId } from "../recipes/new";
import { PromptEditor } from "./PromptEditor";
import type { PlannerCandidate, PlannerPreview, RecipeInfo, TryPlanningResult } from "../types";

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
  /** Ids a deleted recipe left behind in the run history; never handed out again. */
  reserved?: string[];
  busy: boolean;
  error: string | null;
  /** What the planner is doing right now, while busy. */
  progress?: string | null;
  /** What the checker still found after the planner's last try. */
  problems?: string[] | null;
  onCreate: (req: NewRecipeRequest) => void;
  onCancel: () => void;
  onStop?: () => void;
  /** Rendered directly under the Draft it / Create row: a progress window while the planner works. */
  below?: ReactNode;
}

const STARTS: TabItem<NewRecipeMode>[] = [
  { id: "goal", label: "A goal", glyph: "sparkles", hint: "Say what it should do. The planner drafts the steps; you check them on the map." },
  { id: "blank", label: "Blank", glyph: "file", hint: "Reads one file and hands it back. Add steps on the map." },
  { id: "copy", label: "A copy of a recipe", glyph: "copy", hint: "Everything from an existing recipe, under your name." },
];

const VIA_WORD: Record<string, string> = { pinned: "your pick", default: "your default", recommended: "recommended", capability: "Auto" };

/** A trial plan (Try planning) from this screen: running since, or what it came back with. */
type Trial = { modelId: string; since: string } | { modelId: string; result: TryPlanningResult } | { modelId: string; error: string };

export function NewRecipe({ recipes, reserved = [], busy, error, progress, problems, onCreate, onCancel, onStop, below }: Props) {
  const [mode, setMode] = useState<NewRecipeMode>("goal");
  const [sourceId, setSourceId] = useState(recipes[0]?.id ?? "");
  const [name, setName] = useState("");
  const [icon, setIcon] = useState<GlyphName>("recipe");
  const [goal, setGoal] = useState("");
  const [material, setMaterial] = useState("");
  const [model, setModel] = useState<string>("");
  const [preview, setPreview] = useState<PlannerPreview | null>(null);
  const [previewTick, setPreviewTick] = useState(0);
  const [trial, setTrial] = useState<Trial | null>(null);
  const [showPrompt, setShowPrompt] = useState(false);
  const [promptError, setPromptError] = useState<string | null>(null);
  const taken = useMemo(() => [...recipes.map((r) => r.id), ...reserved], [recipes, reserved]);
  const id = uniqueId(slugify(name), taken);
  const ready = name.trim().length > 0 && (mode === "blank" || (mode === "copy" && Boolean(sourceId)) || (mode === "goal" && goal.trim().length > 0));

  useEffect(() => {
    if (mode !== "goal") return;
    let live = true;
    api.previewPlanner(model || null).then((p) => { if (live) setPreview(p); }).catch((e) => { if (live) setPromptError(String(e)); });
    return () => { live = false; };
  }, [mode, model, previewTick]);

  // Measure it: the planner's first call on a fixed sample goal with the chosen model, timed and checked.
  // The engine remembers the result, so the preview is read again afterwards and the cost line updates.
  const trying = trial && "since" in trial ? trial : null;
  const elapsed = useElapsed(trying?.since ?? null, Boolean(trying));
  async function measure(modelId: string) {
    setTrial({ modelId, since: new Date().toISOString() });
    try {
      const result = await api.tryPlanning(modelId);
      setTrial({ modelId, result });
    } catch (e) {
      setTrial({ modelId, error: String(e) });
    } finally {
      setPreviewTick((t) => t + 1);
    }
  }

  const request = (): NewRecipeRequest => ({
    mode, sourceId, id, name: name.trim(), icon,
    ...(mode === "goal" ? { goal: goal.trim(), material: material.trim() || undefined, model: model || null } : {}),
  });
  const submit = () => { if (ready && !busy && !trying) onCreate(request()); };
  const picked = preview?.resolution ?? null;
  // The model the draft would use: the user's pick, else Auto's; with its cost said in words.
  const chosenId = model || picked?.modelId || null;
  const chosen: PlannerCandidate | null = chosenId ? (preview?.candidates ?? []).find((c) => c.modelId === chosenId) ?? null : null;
  const cost = chosen ? costLine(chosen, preview?.memoryBytes) : null;
  // The button says what it still needs, the way Run does in Set up, instead of sitting dim with no reason.
  const needs = !name.trim() ? "needs a name" : mode === "goal" && !goal.trim() ? "needs a goal" : mode === "copy" && !sourceId ? "needs a recipe to copy" : null;
  const planLabel = busy && mode === "goal" ? (progress ?? "Planning…") : busy ? "Creating…" : (mode === "goal" ? "Draft it" : "Create and open its card") + (needs ? ` · ${needs}` : "");

  return (
    <Zone className="accent" tabs={<Tabs label="Start from" items={STARTS} value={mode} onChange={(m) => { if (!busy) setMode(m); }} controls="new-recipe-form" />}>
      <div id="new-recipe-form" role="tabpanel">
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
              <label><strong>What should this recipe do?</strong> <span className="muted">· say what comes in each time and what should come out</span></label>
              <textarea rows={5} value={goal} autoFocus placeholder="Ask me for a folder of PDFs and a question each time, and answer from the documents with the page it came from."
                disabled={busy} onChange={(e) => setGoal(e.target.value)} />
            </div>
            <div className="field">
              <label><strong>Built into the recipe</strong> <span className="muted">· optional · pasted here, it is baked into the recipe's prompts and used every time it cooks</span></label>
              <textarea rows={4} value={material} disabled={busy} placeholder="House-style notes, a fixed list of links, a template to follow. Leave this empty when each run brings its own file or links; say so in the goal instead." onChange={(e) => setMaterial(e.target.value)} />
              <p className="muted material-note">Material pasted here is part of the recipe and travels with it when shared; what varies per run should be asked for, not pasted. Set up lets you change your mind later (make this part of the recipe · ask each run instead).</p>
            </div>
            <div className="field">
              <label><strong>Plan with</strong> <span className="muted">· the model that writes the draft. Auto picks a model that fits in memory, answers in a fixed shape (JSON), and has planned quickly before.</span></label>
              <div className="row" style={{ marginTop: 4 }}>
                <select value={model} disabled={busy || Boolean(trying)} aria-label="Planning model" onChange={(e) => setModel(e.target.value)}>
                  <option value="">Auto{picked && !model ? ` — ${shortModel(picked.modelId)} (${VIA_WORD[picked.via] ?? picked.via})` : ""}</option>
                  {(preview?.candidates ?? []).map((c) => (
                    <option key={c.modelId} value={c.modelId}>{optionLabel(c)}</option>
                  ))}
                </select>
                {picked && <Tag tone={isLocalModel(picked.modelId) ? undefined : "amber"}>{whereWord(picked.modelId)}</Tag>}
                {preview?.problem && <span className="muted">{preview.problem}. Install a model in Settings › Models.</span>}
              </div>
              {chosen && cost && (
                <p className={`plan-cost ${cost.amber ? "amber" : "muted"}`} data-testid="plan-cost">
                  {!model && picked
                    ? <>{shortModel(picked.modelId)}{chosen.sizeBytes != null ? ` · ${gb(chosen.sizeBytes)}` : ""} · {picked.reason}</>
                    : cost.text}
                  {" · "}
                  {trying && trying.modelId === chosen.modelId ? (
                    <span className="plan-trial"><Glyph name="ai" /> Trying… {fmtElapsed(elapsed ?? 0)} <button className="link" onClick={() => api.stopRun("plan").catch(() => {})}><Glyph name="stop" /> Stop</button></span>
                  ) : (
                    <>
                      <button className="link" disabled={busy || Boolean(trying)} onClick={() => measure(chosen.modelId)} title="Run the planner's first call on a sample goal with this model, and time it">
                        {chosen.measuredMs != null ? "measure again" : "measure it"}
                      </button>
                      {trial && !("since" in trial) && trial.modelId === chosen.modelId && (
                        "error" in trial
                          ? <span className="red"> · {/stopped|cancel/i.test(trial.error) ? "stopped" : trial.error}</span>
                          : !trial.result.clean && trial.result.problems[0] ? <span className="muted"> · {trial.result.problems[0]}</span> : null
                      )}
                    </>
                  )}
                </p>
              )}
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
          <button className="primary" style={{ marginTop: 0 }} disabled={!ready || busy || Boolean(trying)} onClick={submit}>
            {busy && mode === "goal" && <Glyph name="ai" />} {planLabel}
          </button>
          {busy && mode === "goal" && onStop ? <button className="stop" onClick={onStop}><Glyph name="stop" /> Stop</button> : <button disabled={busy} onClick={onCancel}>Cancel</button>}
        </div>
        {below}
      </div>
    </Zone>
  );
}
