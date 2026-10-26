import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { ModelsResponse, ModelUsage, RecipeInfo, RecipesResponse, Run, RunSummary, RuntimeStatus, StepRecord } from "./types";
import { ModelsPanel } from "./components/ModelsPanel";
import { RecipePanel } from "./components/RecipePanel";
import { TracePanel } from "./components/TracePanel";
import { ArtifactsPanel } from "./components/ArtifactsPanel";
import { HistoryPanel } from "./components/HistoryPanel";
import { UsagePanel } from "./components/UsagePanel";
import "./App.css";

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

  const refreshModels = useCallback(() => {
    api.listModels().then(setModels).catch((e) => setError(String(e)));
  }, []);
  const refreshRuns = useCallback(() => {
    api.listRuns(20).then(setHistory).catch(() => {});
    api.usage().then(setUsage).catch(() => {});
    api.runtimeStatus().then(setStatus).catch(() => {});
  }, []);

  useEffect(() => {
    api.runtimeStatus().then(setStatus).catch((e) => setError(String(e)));
    api.listRecipes().then(setRecipes).catch((e) => setError(String(e)));
    refreshModels();
    refreshRuns();
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
  }, [refreshModels, refreshRuns]);

  async function startRun(recipe: RecipeInfo, inputs: Record<string, unknown>, pins: Record<string, string>) {
    setRunning(true);
    setError(null);
    setRun(null);
    setRunRecipe(recipe);
    setLiveSteps([]);
    try {
      setRun(await api.runRecipe(recipe.id, inputs, pins));
      refreshRuns();
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  async function openRun(id: string) {
    const r = await api.getRun(id);
    if (r) {
      setRun(r);
      setRunRecipe(recipes?.recipes.find((x) => x.id === r.recipeId) ?? null);
      setLiveSteps([]);
    }
  }

  const steps = run?.steps ?? liveSteps;

  return (
    <main className="shell">
      <header>
        <h1>Stillroom</h1>
        <p className="tagline">Describe the goal. Get the result. Open the door.</p>
      </header>

      <ModelsPanel models={models} onRefresh={refreshModels} />

      {recipes && (
        <RecipePanel recipes={recipes.recipes} models={models?.models ?? []} running={running} onRun={startRun} />
      )}
      {recipes?.errors.map((e) => (
        <pre key={e.path} className="error">{e.path}: {e.error}</pre>
      ))}
      {error && <pre className="error">{error}</pre>}

      <TracePanel run={run} steps={steps} />
      {run && run.status === "succeeded" && <ArtifactsPanel run={run} outputs={runRecipe?.outputs ?? {}} />}

      <HistoryPanel history={history} onOpen={openRun} />
      <UsagePanel usage={usage} />

      <footer className="muted mono">
        {status
          ? `${status.schemaTitle} v0 · tools: ${status.tools.join(", ")} · ${status.runCount} runs · recipes from ${status.recipeRoots.join(" and ")}`
          : "…"}
      </footer>
    </main>
  );
}

export default App;
