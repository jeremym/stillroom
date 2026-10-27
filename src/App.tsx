import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { ModelsResponse, ModelUsage, RecipeInfo, RecipesResponse, Run, RunSummary, RuntimeStatus, StepRecord, WatchRunEvent, WatchView } from "./types";
import { ModelsPanel } from "./components/ModelsPanel";
import { RecipePanel } from "./components/RecipePanel";
import { TracePanel } from "./components/TracePanel";
import { ArtifactsPanel } from "./components/ArtifactsPanel";
import { ReviewPanel } from "./components/ReviewPanel";
import { HistoryPanel } from "./components/HistoryPanel";
import { WatchPanel } from "./components/WatchPanel";
import { UsagePanel } from "./components/UsagePanel";
import { Dial, Empty, Glyph, Logo, Zone, applyLook, fmtElapsed, inputGlyph, loadLook, useElapsed, type DialPlace, type Look } from "./kit";
import "./App.css";

// React Flow is heavy; load it in its own chunk so the rest of the app paints first.
const FlowView = lazy(() => import("./flow/FlowView"));

/* The five places. The Dial moves between them, and moves by itself when a run starts, finishes,
   or needs you, so the lifecycle is something you watch rather than something you learn. */
type Place = "library" | "setup" | "running" | "results" | "door";
const PLACE_LABEL: Record<Place, string> = { library: "Library", setup: "Set up", running: "Running", results: "Results", door: "Open the door" };

function pinKey(id: string) {
  return `stillroom.pins.${id}`;
}
function loadPins(id: string): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(pinKey(id)) ?? "{}");
  } catch {
    return {};
  }
}

const STATUS_WORD: Record<string, string> = { succeeded: "done", failed: "failed", needs_review: "needs you", running: "running" };

/** A value as one short line: a path by its last part, a list by its length, text trimmed. */
function shortValue(v: unknown): string {
  if (typeof v === "string") {
    if (v.startsWith("/")) return v.split("/").pop() ?? v;
    return v.length > 60 ? v.slice(0, 60) + "…" : v;
  }
  if (Array.isArray(v)) return v.length === 1 ? shortValue(v[0]) : `${v.length} items`;
  return JSON.stringify(v);
}

function App() {
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [models, setModels] = useState<ModelsResponse | null>(null);
  const [recipes, setRecipes] = useState<RecipesResponse | null>(null);
  const [liveSteps, setLiveSteps] = useState<StepRecord[]>([]);
  const [run, setRun] = useState<Run | null>(null);
  const [runRecipe, setRunRecipe] = useState<RecipeInfo | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [history, setHistory] = useState<RunSummary[]>([]);
  const [usage, setUsage] = useState<ModelUsage[]>([]);
  const [recipeId, setRecipeId] = useState<string>("");
  const [pins, setPins] = useState<Record<string, string>>({});
  const [watches, setWatches] = useState<WatchView[]>([]);
  const [place, setPlace] = useState<Place>("setup");
  const [look, setLook] = useState<Look>(() => loadLook());
  const recipesRef = useRef<RecipesResponse | null>(null);

  const recipe = recipes?.recipes.find((r) => r.id === recipeId) ?? recipes?.recipes[0] ?? null;
  useEffect(() => {
    if (recipe) setPins(loadPins(recipe.id));
  }, [recipe?.id]);
  useEffect(() => applyLook(look), [look]);

  // Pins are per recipe and remembered on this machine; both the flow and the recipe form change them here.
  const updatePins = useCallback((update: (pins: Record<string, string>) => Record<string, string>) => {
    setPins((p) => {
      const next = update(p);
      if (recipe) {
        try {
          localStorage.setItem(pinKey(recipe.id), JSON.stringify(next));
        } catch { /* per-viewer convenience only */ }
      }
      return next;
    });
  }, [recipe?.id]);

  const refreshModels = useCallback(() => {
    api.listModels().then(setModels).catch((e) => setError(String(e)));
  }, []);
  const openRun = useCallback(async (id: string, go = true) => {
    const r = await api.getRun(id);
    if (r) {
      setRun(r);
      setRunRecipe((current) => current?.id === r.recipeId ? current : recipesRef.current?.recipes.find((x) => x.id === r.recipeId) ?? null);
      setRecipeId(r.recipeId);
      setLiveSteps([]);
      if (go) setPlace("results");
    }
  }, []);
  const refreshRuns = useCallback(() => {
    api.listRuns(20).then(setHistory).catch(() => {});
    api.usage().then(setUsage).catch(() => {});
    api.runtimeStatus().then(setStatus).catch(() => {});
  }, []);
  const refreshWatches = useCallback(() => {
    api.listWatches().then(setWatches).catch(() => {});
  }, []);

  useEffect(() => {
    api.runtimeStatus().then(setStatus).catch((e) => setError(String(e)));
    refreshModels();
    refreshRuns();
    // Load recipes, then show the most recent run so the last result survives a relaunch.
    api
      .listRecipes()
      .then((r) => {
        recipesRef.current = r;
        setRecipes(r);
        return api.listRuns(1);
      })
      .then((latest) => latest[0] && openRun(latest[0].id))
      .catch((e) => setError(String(e)));
    refreshWatches();
    const unlisten = listen<StepRecord>("run:step", (ev) => {
      setLiveSteps((prev) => {
        const next = prev.filter((s) => s.stepId !== ev.payload.stepId);
        next.push(ev.payload);
        return next.sort((a, b) => a.position - b.position);
      });
    });
    // A watch's run shows the same way a run from the Run button does.
    const unlistenWatchRun = listen<WatchRunEvent>("watch:run", (ev) => {
      const w = ev.payload;
      if (w.phase === "started") {
        setRunning(true);
        setError(null);
        setRun(null);
        setLiveSteps([]);
        setRunRecipe(recipesRef.current?.recipes.find((r) => r.id === w.recipeId) ?? null);
        setRecipeId(w.recipeId);
        setPlace("running");
      } else {
        setRunning(false);
        if (w.error && !w.runId) setError(w.error);
        if (w.runId) openRun(w.runId).catch((e) => setError(String(e)));
        refreshRuns();
      }
      refreshWatches();
    });
    const unlistenWatches = listen("watch:changed", () => refreshWatches());
    return () => {
      unlisten.then((f) => f());
      unlistenWatchRun.then((f) => f());
      unlistenWatches.then((f) => f());
    };
  }, [refreshModels, refreshRuns, refreshWatches, openRun]);

  async function startRun(recipe: RecipeInfo, inputs: Record<string, unknown>, pins: Record<string, string>, reuse: boolean) {
    setRunning(true);
    setError(null);
    setRun(null);
    setRunRecipe(recipe);
    setLiveSteps([]);
    setPlace("running");
    try {
      setRun(await api.runRecipe(recipe.id, inputs, pins, reuse));
      refreshRuns();
      setPlace("results");
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  async function watchFolder(recipe: RecipeInfo, input: string, inputs: Record<string, unknown>, pins: Record<string, string>) {
    setError(null);
    try {
      await api.addWatch(recipe.id, input, inputs, pins);
      refreshWatches();
      setPlace("library");
    } catch (e) {
      setError(String(e));
    }
  }

  async function editAndRerun(stepId: string, output: unknown) {
    if (!run) return;
    const priorId = run.id;
    setRunning(true);
    setError(null);
    setRun(null);
    setLiveSteps([]);
    setPlace("running");
    try {
      setRun(await api.rerunWithEdit(priorId, stepId, output, true));
      refreshRuns();
      setPlace("results");
    } catch (e) {
      setError(String(e));
      // Bring the run the user was editing back so they can retry.
      openRun(priorId, false).catch(() => {});
    } finally {
      setRunning(false);
    }
  }

  const steps = run?.steps ?? liveSteps;
  const editedNames = run?.steps.filter((s) => s.edited).map((s) => s.name ?? s.stepId) ?? [];
  const showResult = run && (run.status === "succeeded" || run.status === "needs_review");
  const needsYou = run?.status === "needs_review";

  const runningStep = running ? liveSteps.find((s) => s.status === "running" && !s.parent) ?? liveSteps.find((s) => s.status === "running") : undefined;
  const stepElapsed = useElapsed(runningStep?.startedAt ?? null, running);

  // Progress while running: top-level steps finished over top-level steps declared.
  const progress = useMemo(() => {
    if (!running || !runRecipe) return undefined;
    const total = Math.max(1, runRecipe.steps.length);
    const done = liveSteps.filter((s) => !s.parent && (s.status === "succeeded" || s.status === "skipped")).length;
    return Math.min(0.97, done / total);
  }, [running, runRecipe, liveSteps]);

  const places: DialPlace[] = [
    { id: "library", label: PLACE_LABEL.library },
    { id: "setup", label: PLACE_LABEL.setup },
    { id: "running", label: PLACE_LABEL.running, empty: !running && !run },
    { id: "results", label: PLACE_LABEL.results, empty: !showResult && !running },
    { id: "door", label: PLACE_LABEL.door },
  ];

  const slotCount = (r: RecipeInfo) => Object.keys(r.modelSlots).length;
  const crumb = (() => {
    switch (place) {
      case "library": {
        const n = recipes?.recipes.length ?? 0;
        return <>{n} recipe{n === 1 ? "" : "s"} · {watches.length} watch{watches.length === 1 ? "" : "es"} · {status?.runCount ?? 0} runs</>;
      }
      case "setup":
        return recipe ? <><b>{recipe.name}</b> · {recipe.steps.length} steps · {slotCount(recipe)} model{slotCount(recipe) === 1 ? "" : "s"}, local</> : null;
      case "running": {
        if (running && runRecipe) {
          const done = liveSteps.filter((s) => !s.parent && s.status !== "running").length;
          const current = liveSteps.find((s) => s.status === "running" && !s.parent) ?? liveSteps.find((s) => s.status === "running");
          return <><b>{runRecipe.name}</b> · step {Math.min(done + 1, runRecipe.steps.length)} of {runRecipe.steps.length}{current ? <> · {current.name ?? current.stepId}{stepElapsed != null ? ` · ${fmtElapsed(stepElapsed)}` : ""}</> : null}</>;
        }
        return run ? <><b>{run.recipeName}</b> · finished · {STATUS_WORD[run.status] ?? run.status}</> : <>nothing running</>;
      }
      case "results":
        return run ? <><b>{run.recipeName}</b> · {new Date(run.startedAt).toLocaleString()} · {STATUS_WORD[run.status] ?? run.status}</> : <>no result yet</>;
      case "door":
        return recipe ? <><b>{recipe.name}</b> · {recipe.steps.length} steps · the recipe as a map</> : null;
    }
  })();

  const flowFor = (r: RecipeInfo | null, editable = true) => {
    if (!r) return null;
    const flowRun = run && run.recipeId === r.id ? run : null;
    const records = flowRun?.steps ?? (running && runRecipe?.id === r.id ? liveSteps : []);
    return (
      <Suspense fallback={<section className="panel flow-panel"><p className="muted">Loading the flow…</p></section>}>
        <FlowView
          recipe={r} run={flowRun} records={records} inProgress={running && runRecipe?.id === r.id}
          models={models?.models ?? []} pins={pins} editable={editable}
          onSaved={(saved) => {
            api.listRecipes().then((rr) => {
              recipesRef.current = rr;
              setRecipes(rr);
              setRecipeId(saved.id);
            }).catch((e) => setError(String(e)));
          }}
          onPin={(slot, modelId) => updatePins((p) => {
            const next = { ...p };
            if (modelId) next[slot] = modelId; else delete next[slot];
            return next;
          })}
        />
      </Suspense>
    );
  };

  return (
    <main className="shell">
      <header className="shell-head">
        <nav className="crumbs" aria-label="Where you are">
          <button className="home" onClick={() => setPlace("library")} title="Library: every recipe"><Logo size={30} /></button>
          <span className="sep">›</span>
          {recipe && place !== "library" ? (
            <>
              <button onClick={() => setPlace("setup")} className={place === "setup" ? "here" : ""} title="This recipe"><span className="label">recipe </span>{recipe.name}</button>
              {place !== "setup" && <><span className="sep">›</span><span className="here">{PLACE_LABEL[place]}</span></>}
            </>
          ) : (
            <span className="here">Library</span>
          )}
        </nav>
        <span className="shell-status">
          {models && models.providers.map((p) => (
            <span key={p.id}><span className={p.available ? "dot ok" : "dot bad"} />{p.label} {p.available ? `· ${models.models.length} models` : "· not running"}</span>
          ))}
        </span>
      </header>

      <Dial places={places} current={place} onChange={(id) => setPlace(id as Place)} tone={needsYou ? "amber" : "green"} progress={progress} crumb={crumb} />

      <div className="place">
        {place === "library" && (
          <>
            <p className="tagline">Describe the goal. Get the result. Open the door. <span className="muted">Pick a recipe to start.</span></p>
            <Zone title="Recipes" sub="reusable processes that produce a result" aside={<>{recipes?.recipes.length ?? 0} on the shelf</>}>
              <div className="cards recipe-grid">
                {recipes?.recipes.map((r) => {
                  const fillable = Object.values(r.modelSlots).every((s) => (models?.models ?? []).some((m) => s.requires.every((c) => m.capabilities.includes(c as never))));
                  return (
                    <button key={r.id} className={`card ${recipe?.id === r.id ? "selected" : ""}`} onClick={() => { setRecipeId(r.id); setPlace("setup"); }}>
                      <span className="card-title"><Glyph name="recipe" /> {r.name}</span>
                      {r.description && <span className="dim">{r.description}</span>}
                      <span className="tags">
                        <span className="tag">{r.steps.length} steps</span>
                        {Object.values(r.inputs).filter((i) => i.required !== false).slice(0, 3).map((i, k) => (
                          <span key={k} className="tag"><Glyph name={inputGlyph(i.type)} /> {i.label ?? i.type}</span>
                        ))}
                        <span className="tag">{r.source === "builtin" ? "built-in" : "yours"}</span>
                        {!fillable && <span className="tag amber">needs a model</span>}
                      </span>
                    </button>
                  );
                })}
              </div>
              {recipes?.errors.map((e) => (
                <pre key={e.path} className="error">{e.path}: {e.error}</pre>
              ))}
            </Zone>
            <WatchPanel watches={watches} onChanged={refreshWatches} onOpenRun={openRun} onError={(m) => setError(m)} />
            <HistoryPanel history={history} onOpen={openRun} />
            <UsagePanel usage={usage} />
            <ModelsPanel models={models} onRefresh={refreshModels} />
            <Zone title="Look" sub="theme, text size">
              <div className="settings-row">
                <span className="label">Theme</span>
                <span className="segmented" role="group" aria-label="Theme">
                  {(["green", "amber", "paper"] as const).map((t) => (
                    <button key={t} aria-pressed={look.theme === t} onClick={() => setLook({ ...look, theme: t })}>{t}</button>
                  ))}
                </span>
                <span className="label">Text size</span>
                <span className="segmented" role="group" aria-label="Text size">
                  {(["normal", "larger", "largest"] as const).map((s) => (
                    <button key={s} aria-pressed={look.size === s} onClick={() => setLook({ ...look, size: s })}>{s}</button>
                  ))}
                </span>
                <span className="label">Scanlines</span>
                <span>
                  <button className="toggle" aria-pressed={look.scanlines} onClick={() => setLook({ ...look, scanlines: !look.scanlines })}>
                    <span className={look.scanlines ? "on" : ""}>on</span><span className={look.scanlines ? "" : "on"}>off</span>
                  </button>
                  <span className="muted"> · faint lines over the screen, off unless you like them</span>
                </span>
              </div>
            </Zone>
          </>
        )}

        {place === "setup" && recipes && recipe && (
          <RecipePanel recipes={recipes.recipes} recipe={recipe} onSelect={setRecipeId} pins={pins} onPins={updatePins}
            models={models?.models ?? []} running={running} onRun={startRun} onWatch={watchFolder} />
        )}

        {place === "running" && (
          running || run ? (
            <>
              {flowFor(runRecipe ?? recipe, false)}
              <TracePanel run={run} steps={steps} />
            </>
          ) : (
            <Zone>
              <Empty word="Nothing running">
                <p>Set up a recipe and press Run. This place shows each step as it happens.</p>
                <button onClick={() => setPlace("setup")}>Go to Set up</button>
              </Empty>
            </Zone>
          )
        )}

        {place === "results" && (
          showResult ? (
            <>
              <Zone title={run.status === "needs_review" ? "Waiting for you" : `What ${run.recipeName} made`} sub={run.status === "needs_review" ? "the run is paused at a review step" : "this run's results"}
                aside={<>{new Date(run.startedAt).toLocaleString()}{run.finishedAt ? ` · took ${fmtElapsed(Math.round((new Date(run.finishedAt).getTime() - new Date(run.startedAt).getTime()) / 1000))}` : ""}</>}>
                <div className="result-facts">
                  <span className="label">From</span>
                  <span>
                    {Object.entries(run.inputs ?? {}).filter(([, v]) => v != null && v !== "").length === 0 ? <span className="muted">no inputs</span> :
                      Object.entries(run.inputs ?? {}).filter(([, v]) => v != null && v !== "").map(([k, v]) => (
                        <span key={k} className="fact"><Glyph name={inputGlyph(runRecipe?.inputs[k]?.type ?? "text")} /> <span className="muted">{runRecipe?.inputs[k]?.label ?? k}:</span> <span className="mono">{shortValue(v)}</span></span>
                      ))}
                  </span>
                  <span className="label">Made by</span>
                  <span>
                    {Object.values(run.resolutions ?? {}).length === 0 ? <span className="muted">programs only, no model</span> :
                      Object.values(run.resolutions ?? {}).map((r) => (
                        <span key={r.slot} className="fact"><Glyph name="ai" /> <span className="muted">{r.slot}:</span> <span className="mono">{r.modelId}</span> <span className="tag">local</span></span>
                      ))}
                    {" "}<span className="muted">· {run.steps.filter((s) => !s.parent).length} steps{run.steps.some((s) => s.reusedFrom) ? `, ${run.steps.filter((s) => s.reusedFrom).length} reused from earlier runs` : ""}</span>
                  </span>
                  {(editedNames.length > 0 || run.trigger) && <>
                    <span className="label">Note</span>
                    <span className="muted">
                      {editedNames.length > 0 && <>Includes your {run.steps.some((s) => s.edited && s.kind === "review") ? "decision on" : "edit to"} <strong>{editedNames.join(", ")}</strong>. </>}
                      {run.trigger && <>Started by the folder watch{run.trigger.changed.length > 0 ? <> after <strong>{run.trigger.changed.slice(0, 3).join(", ")}{run.trigger.changed.length > 3 ? ` and ${run.trigger.changed.length - 3} more` : ""}</strong> landed</> : " (Run now)"}.</>}
                    </span>
                  </>}
                </div>
                <p className="muted result-hint">Each result below is a tab. Edit one and only the steps after it run again.</p>
              </Zone>
              {run.status === "needs_review" && (
                <ReviewPanel run={run} steps={runRecipe?.steps ?? []} busy={running} onDecide={editAndRerun} />
              )}
              <ArtifactsPanel run={run} outputs={runRecipe?.outputs ?? {}} steps={runRecipe?.steps ?? []} busy={running} onEdit={editAndRerun} />
              <p className="result-line">
                How this was made: <button className="link" onClick={() => setPlace("running")}>the trace (every step, model, and timing)</button>
                {" · "}
                <button className="link" onClick={() => { if (runRecipe) setRecipeId(runRecipe.id); setPlace("door"); }}>the map (open the door)</button>
              </p>
            </>
          ) : running ? (
            <Zone>
              <Empty word="Running">
                <p>The result lands here when the run finishes.</p>
                <button onClick={() => setPlace("running")}>Watch it run</button>
              </Empty>
            </Zone>
          ) : run && run.status === "failed" ? (
            <>
              <Zone title="This run failed" className="accent">
                <pre className="error">{run.error ?? run.steps.find((s) => s.error)?.error ?? "no reason recorded"}</pre>
                <button onClick={() => setPlace("setup")}>Back to Set up</button>
              </Zone>
              <TracePanel run={run} steps={steps} />
            </>
          ) : (
            <Zone>
              <Empty word="No result yet">
                <p>Run a recipe and what it makes shows up here: notes, slides, tables, audio.</p>
                <button onClick={() => setPlace("setup")}>Go to Set up</button>
              </Empty>
            </Zone>
          )
        )}

        {place === "door" && flowFor(recipe)}

        {error && <pre className="error">{error}</pre>}
      </div>

      <footer>
        {status
          ? `${status.schemaTitle} v0 · tools: ${status.tools.join(", ")} · ${status.runCount} runs · recipes from ${status.recipeRoots.join(" and ")}`
          : "…"}
      </footer>
    </main>
  );
}

export default App;
