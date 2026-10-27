import { Suspense, lazy, useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { ModelsResponse, ModelUsage, RecipeInfo, RecipesResponse, Run, RunSummary, RuntimeStatus, StepRecord } from "./types";
import { ModelsPanel } from "./components/ModelsPanel";
import { RecipePanel } from "./components/RecipePanel";
import { TracePanel } from "./components/TracePanel";
import { ArtifactsPanel } from "./components/ArtifactsPanel";
import { ReviewPanel } from "./components/ReviewPanel";
import { HistoryPanel } from "./components/HistoryPanel";
import { UsagePanel } from "./components/UsagePanel";
import "./App.css";

// React Flow is heavy; load it in its own chunk so the rest of the app paints first.
const FlowView = lazy(() => import("./flow/FlowView"));

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
  const recipesRef = useRef<RecipesResponse | null>(null);

  const recipe = recipes?.recipes.find((r) => r.id === recipeId) ?? recipes?.recipes[0] ?? null;
  useEffect(() => {
    if (recipe) setPins(loadPins(recipe.id));
  }, [recipe?.id]);
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
  const openRun = useCallback(async (id: string) => {
    const r = await api.getRun(id);
    if (r) {
      setRun(r);
      setRunRecipe((current) => current?.id === r.recipeId ? current : recipesRef.current?.recipes.find((x) => x.id === r.recipeId) ?? null);
      setRecipeId(r.recipeId);
      setLiveSteps([]);
    }
  }, []);
  const refreshRuns = useCallback(() => {
    api.listRuns(20).then(setHistory).catch(() => {});
    api.usage().then(setUsage).catch(() => {});
    api.runtimeStatus().then(setStatus).catch(() => {});
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
    const unlisten = listen<StepRecord>("run:step", (ev) => {
      setLiveSteps((prev) => {
        const next = prev.filter((s) => s.stepId !== ev.payload.stepId);
        next.push(ev.payload);
        return next.sort((a, b) => a.position - b.position);
      });
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [refreshModels, refreshRuns, openRun]);

  async function startRun(recipe: RecipeInfo, inputs: Record<string, unknown>, pins: Record<string, string>, reuse: boolean) {
    setRunning(true);
    setError(null);
    setRun(null);
    setRunRecipe(recipe);
    setLiveSteps([]);
    try {
      setRun(await api.runRecipe(recipe.id, inputs, pins, reuse));
      refreshRuns();
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  async function editAndRerun(stepId: string, output: unknown) {
    if (!run) return;
    const priorId = run.id;
    setRunning(true);
    setError(null);
    setRun(null);
    setLiveSteps([]);
    try {
      setRun(await api.rerunWithEdit(priorId, stepId, output, true));
      refreshRuns();
    } catch (e) {
      setError(String(e));
      // Bring the run the user was editing back so they can retry.
      openRun(priorId).catch(() => {});
    } finally {
      setRunning(false);
    }
  }

  const steps = run?.steps ?? liveSteps;
  // The flow draws the selected recipe and paints the current run on it only if it is a run of that recipe.
  const flowRun = run && recipe && run.recipeId === recipe.id ? run : null;
  const flowRecords = flowRun?.steps ?? (running && runRecipe?.id === recipe?.id ? liveSteps : []);
  const editedNames = run?.steps.filter((s) => s.edited).map((s) => s.name ?? s.stepId) ?? [];
  const showResult = run && (run.status === "succeeded" || run.status === "needs_review");

  return (
    <main className="shell">
      <header>
        <h1>Stillroom</h1>
        <p className="tagline">Describe the goal. Get the result. Open the door.</p>
      </header>

      {recipe && (
        <Suspense fallback={<section className="panel flow-panel"><p className="muted">Loading the flow…</p></section>}>
          <FlowView
            recipe={recipe} run={flowRun} records={flowRecords} inProgress={running && runRecipe?.id === recipe.id}
            models={models?.models ?? []} pins={pins}
            onSaved={(saved) => {
              api.listRecipes().then((r) => {
                recipesRef.current = r;
                setRecipes(r);
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
      )}

      {showResult && (
        <p className="muted">
          {run.status === "needs_review" ? "Waiting for you" : "Latest result"} · {run.recipeName} · {new Date(run.startedAt).toLocaleString()}
          {editedNames.length > 0 && <> · includes your {run.steps.some((s) => s.edited && s.kind === "review") ? "decision on" : "edit to"} <strong>{editedNames.join(", ")}</strong></>}
          <span className="muted"> · the recipe panel below starts a new run</span>
        </p>
      )}
      {run && run.status === "needs_review" && (
        <ReviewPanel run={run} steps={runRecipe?.steps ?? []} busy={running} onDecide={editAndRerun} />
      )}
      {showResult && (
        <ArtifactsPanel run={run} outputs={runRecipe?.outputs ?? {}} steps={runRecipe?.steps ?? []} busy={running} onEdit={editAndRerun} />
      )}
      <TracePanel run={run} steps={steps} />

      {recipes && recipe && (
        <RecipePanel recipes={recipes.recipes} recipe={recipe} onSelect={setRecipeId} pins={pins} onPins={updatePins}
          models={models?.models ?? []} running={running} onRun={startRun} />
      )}
      {recipes?.errors.map((e) => (
        <pre key={e.path} className="error">{e.path}: {e.error}</pre>
      ))}
      {error && <pre className="error">{error}</pre>}

      <HistoryPanel history={history} onOpen={openRun} />
      <UsagePanel usage={usage} />
      <ModelsPanel models={models} onRefresh={refreshModels} />

      <footer className="muted mono">
        {status
          ? `${status.schemaTitle} v0 · tools: ${status.tools.join(", ")} · ${status.runCount} runs · recipes from ${status.recipeRoots.join(" and ")}`
          : "…"}
      </footer>
    </main>
  );
}

export default App;
