import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { ModelsResponse, ModelUsage, RecipeInfo, RecipesResponse, Run, RunSummary, RuntimeStatus, Settings, StepRecord, WatchRunEvent, WatchView } from "./types";
import { RecipePanel } from "./components/RecipePanel";
import { TracePanel } from "./components/TracePanel";
import { Teletype } from "./components/Teletype";
import { ArtifactsPanel } from "./components/ArtifactsPanel";
import { ReviewPanel } from "./components/ReviewPanel";
import { HistoryPanel } from "./components/HistoryPanel";
import { WatchPanel } from "./components/WatchPanel";
import { UsagePanel } from "./components/UsagePanel";
import { RecipePage } from "./components/RecipePage";
import { NewRecipe, type NewRecipeRequest } from "./components/NewRecipe";
import { Welcome } from "./components/Welcome";
import { blankRecipe, copyRecipe, slugify, uniqueId } from "./recipes/new";
import { Glyph, Logo, inputGlyph, isGlyphName } from "./brand";
import { Dial, Empty, Pill, Rail, Zone, applyLook, fmtElapsed, loadLook, useElapsed, type DialPlace, type Look } from "@overtone-foundry/app-kit";
import "./App.css";

// React Flow is heavy; load it in its own chunk so the rest of the app paints first.
const FlowView = lazy(() => import("./flow/FlowView"));

/* Places. Library is home. Opening a recipe puts the Dial on screen with that recipe's five places;
   the Dial turns by itself when a run starts, finishes, or needs you. Settings and New recipe sit
   outside the Dial, reached from the Rail or the menu. */
type Place = "welcome" | "library" | "new" | "recipe" | "setup" | "running" | "results" | "door";
type RecipePlace = Exclude<Place, "welcome" | "library" | "new">;
const PLACE_LABEL: Record<Place, string> = {
  welcome: "Welcome", library: "Library", new: "New recipe",
  recipe: "Recipe", setup: "Set up", running: "Running", results: "Results", door: "Recipe card",
};
const RECIPE_PLACES: RecipePlace[] = ["recipe", "setup", "running", "results", "door"];
const isRecipePlace = (p: Place): p is RecipePlace => (RECIPE_PLACES as Place[]).includes(p);

const STATUS_WORD: Record<string, string> = { succeeded: "done", failed: "failed", cancelled: "stopped", needs_review: "needs you", running: "running" };
const SIZES = ["normal", "larger", "largest"] as const;

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
  const [settings, setSettingsState] = useState<Settings>({});
  const [watches, setWatches] = useState<WatchView[]>([]);
  const [place, setPlace] = useState<Place>("library");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [freshId, setFreshId] = useState<string | null>(null);
  const [helpOpen, setHelpOpen] = useState(false);
  const [confirmStop, setConfirmStop] = useState(false);
  // The welcome shows until it has been finished or skipped once (firstRun absent means never shown).
  const [firstRunHint, setFirstRunHint] = useState(false);
  const [stopping, setStopping] = useState(false);
  const recipesRef = useRef<RecipesResponse | null>(null);
  const placeRef = useRef<Place>("library");
  placeRef.current = place;

  const recipe = recipes?.recipes.find((r) => r.id === recipeId) ?? null;
  // Pins live in Settings (the Rust store) per recipe, so watches and the Settings window see the same ones.
  const pins = useMemo(() => settings.pins?.[recipeId] ?? {}, [settings.pins, recipeId]);
  const settingsRef = useRef<Settings>({});
  settingsRef.current = settings;
  // The look (theme, text size) is a setting too; local storage only keeps the first paint right.
  const look = useMemo<Look>(() => {
    const base = loadLook();
    return { theme: settings.look?.theme ?? base.theme, size: settings.look?.size ?? base.size, scanlines: settings.look?.scanlines ?? base.scanlines };
  }, [settings.look]);
  useEffect(() => applyLook(look), [look]);
  const saveSettings = useCallback((patch: Record<string, unknown>) => {
    api.setSettings(patch).then(setSettingsState).catch((e) => setError(String(e)));
  }, []);
  const setLook = useCallback((update: Look | ((l: Look) => Look)) => {
    const cur = (() => { const b = loadLook(); const l = settingsRef.current.look; return { theme: l?.theme ?? b.theme, size: l?.size ?? b.size, scanlines: l?.scanlines ?? b.scanlines }; })();
    saveSettings({ look: typeof update === "function" ? update(cur) : update });
  }, [saveSettings]);

  // A recipe place with no recipe open falls back to the Library.
  useEffect(() => {
    if (isRecipePlace(place) && recipes && !recipe) setPlace("library");
  }, [place, recipe, recipes]);

  // Both the flow and the recipe form change pins here; they go to the store.
  const updatePins = useCallback((update: (pins: Record<string, string>) => Record<string, string>) => {
    if (!recipeId) return;
    const all = settingsRef.current.pins ?? {};
    const next = update(all[recipeId] ?? {});
    saveSettings({ pins: { ...all, [recipeId]: next } });
  }, [recipeId, saveSettings]);

  const refreshModels = useCallback(() => {
    api.listModels().then(setModels).catch((e) => setError(String(e)));
  }, []);
  const refreshRecipes = useCallback(async () => {
    const r = await api.listRecipes();
    recipesRef.current = r;
    setRecipes(r);
    return r;
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
    api.listRuns(50).then(setHistory).catch(() => {});
    api.usage().then(setUsage).catch(() => {});
    api.runtimeStatus().then(setStatus).catch(() => {});
  }, []);
  const refreshWatches = useCallback(() => {
    api.listWatches().then(setWatches).catch(() => {});
  }, []);

  const finishWelcome = useCallback((recipeId?: string) => {
    saveSettings({ firstRun: false });
    if (recipeId) {
      setFirstRunHint(true);
      setRecipeId(recipeId);
      setFreshId(null);
      setPlace("setup");
    } else {
      setPlace("library");
    }
  }, [saveSettings]);

  const openRecipe = useCallback((id: string, where: RecipePlace = "recipe") => {
    setRecipeId(id);
    setFreshId(null);
    setPlace(where);
  }, []);

  useEffect(() => {
    api.getSettings().then((s) => {
      setSettingsState(s);
      if (s.firstRun !== false) setPlace("welcome");
    }).catch((e) => setError(String(e)));
    api.runtimeStatus().then(setStatus).catch((e) => setError(String(e)));
    refreshModels();
    refreshRuns();
    // Load recipes, then remember the most recent run so its result is one click away.
    refreshRecipes()
      .then(() => api.listRuns(1))
      .then((latest) => latest[0] && openRun(latest[0].id, false))
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
    // Settings change in the Settings window too; the model list changes when Ollama's address does.
    const unlistenSettings = listen<Settings>("settings:changed", (ev) => {
      const before = settingsRef.current.ollama?.baseUrl;
      setSettingsState(ev.payload);
      if (ev.payload.ollama?.baseUrl !== before) refreshModels();
    });
    return () => {
      unlisten.then((f) => f());
      unlistenWatchRun.then((f) => f());
      unlistenWatches.then((f) => f());
      unlistenSettings.then((f) => f());
    };
  }, [refreshModels, refreshRuns, refreshWatches, refreshRecipes, openRun]);

  // The native menu: every item sends its id here. Keyboard shortcuts live on the menu items.
  const onMenu = useCallback((id: string) => {
    const goRecipe = (p: RecipePlace) => { if (recipesRef.current?.recipes.length) setPlace((cur) => (isRecipePlace(cur) || recipeId ? p : cur)); };
    switch (id) {
      case "about": case "settings": api.openSettings().catch((e) => setError(String(e))); break;
      case "new-recipe": setCreateError(null); setPlace("new"); break;
      case "reveal-recipes": api.revealRecipesFolder().catch((e) => setError(String(e))); break;
      case "place-library": setPlace("library"); break;
      case "place-recipe": goRecipe("recipe"); break;
      case "place-setup": goRecipe("setup"); break;
      case "place-running": goRecipe("running"); break;
      case "place-results": goRecipe("results"); break;
      case "place-door": goRecipe("door"); break;
      case "theme-green": setLook((l) => ({ ...l, theme: "green" })); break;
      case "theme-amber": setLook((l) => ({ ...l, theme: "amber" })); break;
      case "theme-paper": setLook((l) => ({ ...l, theme: "paper" })); break;
      case "text-larger": setLook((l) => ({ ...l, size: SIZES[Math.min(SIZES.length - 1, SIZES.indexOf(l.size) + 1)] })); break;
      case "text-smaller": setLook((l) => ({ ...l, size: SIZES[Math.max(0, SIZES.indexOf(l.size) - 1)] })); break;
      case "run":
        if (placeRef.current === "setup") window.dispatchEvent(new CustomEvent("stillroom:run"));
        else goRecipe("setup");
        break;
      case "watch-folder": goRecipe("setup"); break;
      case "help": case "shortcuts": setHelpOpen(true); break;
    }
  }, [recipeId, setLook]);
  useEffect(() => {
    const un = listen<string>("menu", (ev) => onMenu(ev.payload));
    return () => { un.then((f) => f()); };
  }, [onMenu]);
  // Escape closes the Help sheet and an open stop confirmation.
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setHelpOpen(false);
      setConfirmStop(false);
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);

  // Outside Tauri (plain browser dev) the menu does not exist; a keyboard fallback keeps the shortcuts.
  useEffect(() => {
    if ("__TAURI_INTERNALS__" in window) return;
    const h = (e: KeyboardEvent) => {
      if (!e.metaKey || e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      const map: Record<string, string> = { ",": "settings", n: "new-recipe", "0": "place-library", "1": "place-recipe", "2": "place-setup", "3": "place-running", "4": "place-results", "5": "place-door", r: "run", "/": "shortcuts" };
      const id = map[e.key];
      if (id) { e.preventDefault(); onMenu(id); }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [onMenu]);

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
      if (firstRunHint) setTimeout(() => setFirstRunHint(false), 60_000);
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
      setPlace("recipe");
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

  async function createRecipe(req: NewRecipeRequest) {
    setCreating(true);
    setCreateError(null);
    try {
      let draft;
      if (req.mode === "copy" && req.sourceId) {
        const src = await api.readRecipeDraft(req.sourceId);
        draft = copyRecipe(src.draft, req.id, req.name, req.icon, src.source === "builtin");
      } else {
        draft = blankRecipe(req.id, req.name, req.icon);
      }
      const saved = await api.saveRecipeDraft(draft);
      await refreshRecipes();
      setRecipeId(saved.recipe.id);
      setFreshId(saved.recipe.id);
      setPlace("door");
    } catch (e) {
      setCreateError(String(e));
    } finally {
      setCreating(false);
    }
  }

  async function duplicateRecipe(r: RecipeInfo) {
    setError(null);
    try {
      const src = await api.readRecipeDraft(r.id);
      const taken = recipesRef.current?.recipes.map((x) => x.id) ?? [];
      const id = uniqueId(`${slugify(r.name)}-copy`, taken);
      const saved = await api.saveRecipeDraft(copyRecipe(src.draft, id, `${r.name} (copy)`, undefined, src.source === "builtin"));
      await refreshRecipes();
      openRecipe(saved.recipe.id);
    } catch (e) {
      setError(String(e));
    }
  }

  async function renameRecipe(r: RecipeInfo, name: string) {
    setError(null);
    try {
      const src = await api.readRecipeDraft(r.id);
      await api.saveRecipeDraft({ yaml: { ...src.draft.yaml, name }, files: src.draft.files });
      await refreshRecipes();
    } catch (e) {
      setError(String(e));
    }
  }

  async function deleteRecipe(r: RecipeInfo) {
    setError(null);
    try {
      await api.deleteRecipe(r.id);
      await refreshRecipes();
      refreshWatches();
      setRecipeId("");
      setPlace("library");
    } catch (e) {
      setError(String(e));
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

  // The run shown on the Dial places belongs to the open recipe; another recipe's run stays out of the way.
  const runIsHere = Boolean(run && recipe && run.recipeId === recipe.id);
  const runningHere = running && runRecipe?.id === recipe?.id;
  const places: DialPlace[] = [
    { id: "recipe", label: PLACE_LABEL.recipe },
    { id: "setup", label: PLACE_LABEL.setup },
    { id: "running", label: PLACE_LABEL.running, empty: !runningHere && !runIsHere },
    { id: "results", label: PLACE_LABEL.results, empty: !(runIsHere && showResult) && !runningHere },
    { id: "door", label: PLACE_LABEL.door },
  ];

  const crumb = (() => {
    if (!recipe) return null;
    switch (place) {
      case "recipe":
        return <><b>{recipe.name}</b> · {recipe.steps.length} steps · {Object.keys(recipe.modelSlots).length} model{Object.keys(recipe.modelSlots).length === 1 ? "" : "s"}, local</>;
      case "setup":
        return <>Set up a run of <b>{recipe.name}</b></>;
      case "running": {
        if (runningHere && runRecipe) {
          const done = liveSteps.filter((s) => !s.parent && s.status !== "running").length;
          const current = runningStep;
          return <><b>{runRecipe.name}</b> · step {Math.min(done + 1, runRecipe.steps.length)} of {runRecipe.steps.length}{current ? <> · {current.name ?? current.stepId}{stepElapsed != null ? ` · ${fmtElapsed(stepElapsed)}` : ""}</> : null}</>;
        }
        return runIsHere && run ? <><b>{run.recipeName}</b> · finished · {STATUS_WORD[run.status] ?? run.status}</> : <>nothing running</>;
      }
      case "results":
        return runIsHere && run ? <><b>{run.recipeName}</b> · {new Date(run.startedAt).toLocaleString()} · {STATUS_WORD[run.status] ?? run.status}</> : <>no result yet</>;
      case "door":
        return <><b>{recipe.name}</b> · the recipe written out, editable</>;
      default:
        return null;
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
          models={models?.models ?? []} pins={pins} editable={editable} startInEdit={editable && freshId === r.id}
          onSaved={(saved) => {
            setFreshId(null);
            refreshRecipes().then(() => setRecipeId(saved.id)).catch((e) => setError(String(e)));
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

  const recipeIcon = recipe && isGlyphName(recipe.icon) ? recipe.icon : "recipe";
  const runsOfThis = run ? history.filter((h) => h.recipeId === run.recipeId) : [];

  return (
    <>
      <Rail
        left={
          <nav className="crumbs" aria-label="Where you are">
            <button className="home" onClick={() => setPlace("library")} title="Library: every recipe"><Logo size={28} /></button>
            {place === "library" && <><span className="sep">›</span><span className="here">Library</span></>}
            {(place === "new" || place === "welcome") && <><span className="sep">›</span><span className="here">{PLACE_LABEL[place]}</span></>}
            {isRecipePlace(place) && recipe && (
              <>
                <span className="sep">›</span>
                <button onClick={() => setPlace("recipe")} className={place === "recipe" ? "here" : ""} title="This recipe's page">
                  <Glyph name={recipeIcon} /> <span className="label">recipe </span>{recipe.name}
                </button>
                {place !== "recipe" && <><span className="sep">›</span><span className="here">{PLACE_LABEL[place]}</span></>}
              </>
            )}
          </nav>
        }
        right={
          <>
            {models && models.providers.map((p) => (
              <Pill key={p.id} ok={p.available} onClick={() => api.openSettings().catch((e) => setError(String(e)))} title="Models and providers (Settings)">
                {p.label} · {p.available ? `${models.models.length} models` : "not running"}
              </Pill>
            ))}
            <button className="icon" onClick={() => { setCreateError(null); setPlace("new"); }} title="New recipe (⌘N)"><Glyph name="plus" /> New recipe</button>
            <button className="icon" onClick={() => api.openSettings().catch((e) => setError(String(e)))} title="Settings (⌘,)" aria-label="Settings"><Glyph name="settings" /></button>
            <button className="icon" onClick={() => setHelpOpen((o) => !o)} title="Help and shortcuts (⌘/)" aria-label="Help" aria-pressed={helpOpen}><Glyph name="help" /></button>
          </>
        }
      />

      <main className="shell">
        {isRecipePlace(place) && recipe && (
          <Dial places={places} current={place} onChange={(id) => setPlace(id as Place)} tone={needsYou && runIsHere ? "amber" : "green"} progress={runningHere ? progress : undefined} crumb={crumb} />
        )}

        <div className="place">
          {helpOpen && (
            <Zone title="Help" sub="how Stillroom is laid out" aside={<button className="quiet" onClick={() => setHelpOpen(false)}>Close</button>}>
              <div className="facts">
                <span className="label">Library</span><span>Every recipe, every run, your watches. The logo at the top left always brings you back.</span>
                <span className="label">Recipe</span><span>Pick one and the Dial appears: <b>Recipe</b> (what it does, its runs) · <b>Set up</b> (inputs, models, Run) · <b>Running</b> (each step as it happens) · <b>Results</b> (what it made) · <b>Recipe card</b> (the recipe written out as a map you can edit).</span>
                <span className="label">Colours</span><span>Green is the machine. Paper is what it made for you. Amber means it needs you, or something would leave this Mac. Red is a failure.</span>
                <span className="label">Keys</span><span className="mono">⌘0 Library · ⌘1…⌘5 the Dial · ⌘N New recipe · ⌘R Run · ⌘, Settings · ⌘/ this help</span>
              </div>
            </Zone>
          )}

          {place === "welcome" && (
            <Welcome models={models} settings={settings} recipes={recipes?.recipes ?? []} onSave={saveSettings}
              onOpenSettings={() => api.openSettings().catch((e) => setError(String(e)))} onRefreshModels={refreshModels}
              onPickRecipe={(id) => finishWelcome(id)} onSkip={() => finishWelcome()} />
          )}

          {place === "library" && (
            <>
              <p className="tagline">Describe the goal. Get the result. See how it was made. <span className="muted">Pick a recipe to start, or make one.</span></p>
              <Zone title="Recipes" sub="reusable processes that produce a result" aside={<><span>{recipes?.recipes.length ?? 0} on the shelf</span><button className="quiet" onClick={() => { setCreateError(null); setPlace("new"); }}><Glyph name="plus" /> New recipe</button></>}>
                <div className="cards recipe-grid">
                  {recipes?.recipes.map((r) => {
                    const fillable = Object.values(r.modelSlots).every((s) => (models?.models ?? []).some((m) => s.requires.every((c) => m.capabilities.includes(c as never))));
                    const lastRun = history.find((h) => h.recipeId === r.id);
                    return (
                      <button key={r.id} className="card" onClick={() => openRecipe(r.id)}>
                        <span className="card-title"><Glyph name={isGlyphName(r.icon) ? r.icon : "recipe"} /> {r.name}</span>
                        {r.description && <span className="dim">{r.description}</span>}
                        <span className="tags">
                          <span className="tag">{r.steps.length} steps</span>
                          {Object.values(r.inputs).filter((i) => i.required !== false).slice(0, 3).map((i, k) => (
                            <span key={k} className="tag"><Glyph name={inputGlyph(i.type)} /> {i.label ?? i.type}</span>
                          ))}
                          <span className="tag">{r.source === "builtin" ? "built-in" : "yours"}</span>
                          {!fillable && <span className="tag amber">needs a model</span>}
                          {lastRun && <span className="tag">last run {new Date(lastRun.startedAt).toLocaleDateString()}</span>}
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
            </>
          )}

          {place === "new" && recipes && (
            <NewRecipe recipes={recipes.recipes} busy={creating} error={createError} onCreate={createRecipe}
              onCancel={() => setPlace(recipe ? "recipe" : "library")} />
          )}

          {place === "recipe" && recipe && (
            <RecipePage recipe={recipe} models={models?.models ?? []} history={history} watches={watches}
              map={flowFor(recipe, false)}
              onSetup={() => setPlace("setup")} onDoor={() => setPlace("door")} onOpenRun={openRun}
              onDuplicate={() => duplicateRecipe(recipe)} onRename={(n) => renameRecipe(recipe, n)} onDelete={() => deleteRecipe(recipe)}
              onWatchesChanged={refreshWatches} onError={(m) => setError(m)} />
          )}

          {place === "setup" && recipes && recipe && firstRunHint && (
            <p className="result-line hint-line"><Glyph name="sparkles" /> Step 3 of 3: choose what the recipe reads, then press <strong>Run</strong>. The Dial above follows the run to Results on its own.</p>
          )}
          {place === "setup" && recipes && recipe && (
            <RecipePanel recipes={recipes.recipes} recipe={recipe} onSelect={(id) => openRecipe(id, "setup")} pins={pins} onPins={updatePins}
              models={models?.models ?? []} running={running} onRun={startRun} onWatch={watchFolder} />
          )}

          {place === "running" && (
            runningHere || runIsHere ? (
              <>
                {runningHere && (
                  <div className="row stop-row">
                    {confirmStop
                      ? <><button className="danger" disabled={stopping} onClick={() => { setStopping(true); api.stopRun().catch((e) => setError(String(e))).finally(() => { setStopping(false); setConfirmStop(false); }); }}>{stopping ? "Stopping…" : "Really stop"}</button><button onClick={() => setConfirmStop(false)}>Keep running</button></>
                      : <button className="stop" onClick={() => setConfirmStop(true)}><Glyph name="stop" /> Stop</button>}
                    <span className="muted">Stop ends the run now: the step in progress is abandoned, and nothing already written is undone.</span>
                  </div>
                )}
                {flowFor(runRecipe ?? recipe, false)}
                <Zone title="As it happens" sub="each step typed as it starts and finishes (the trace)">
                  <Teletype records={runIsHere || runningHere ? steps : []} live={runningHere} />
                  <details className="trace-table">
                    <summary className="muted">The same facts as a table</summary>
                    <TracePanel run={runIsHere ? run : null} steps={runIsHere || runningHere ? steps : []} />
                  </details>
                </Zone>
              </>
            ) : (
              <Zone>
                <Empty word="Nothing running">
                  <p>Set up a run and press Run. This place shows each step as it happens.</p>
                  <button onClick={() => setPlace("setup")}>Go to Set up</button>
                </Empty>
              </Zone>
            )
          )}

          {place === "results" && (
            runIsHere && run && showResult ? (
              <>
                <Zone title={run.status === "needs_review" ? "Waiting for you" : `What ${run.recipeName} made`} sub={run.status === "needs_review" ? "the run is paused at a review step" : "this run's results"}
                  aside={<>
                    {runsOfThis.length > 1 && (
                      <label className="run-pick">
                        <span className="muted">run</span>
                        <select value={run.id} onChange={(e) => openRun(e.target.value)} aria-label="Which run">
                          {runsOfThis.map((h) => <option key={h.id} value={h.id}>{new Date(h.startedAt).toLocaleString()} · {STATUS_WORD[h.status] ?? h.status}</option>)}
                        </select>
                      </label>
                    )}
                    <span>{new Date(run.startedAt).toLocaleString()}{run.finishedAt ? ` · took ${fmtElapsed(Math.round((new Date(run.finishedAt).getTime() - new Date(run.startedAt).getTime()) / 1000))}` : ""}</span>
                  </>}>
                  <div className="facts">
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
                {firstRunHint && (
                  <p className="result-line hint-line"><Glyph name="sparkles" /> That was your first run. The <button className="link" onClick={() => setPlace("door")}>Recipe card</button> shows how it was made and lets you change it; <strong>Set up</strong> runs it again on something else.</p>
                )}
                <p className="result-line">
                  How this was made: <button className="link" onClick={() => setPlace("running")}>the trace (every step, model, and timing)</button>
                  {" · "}
                  <button className="link" onClick={() => setPlace("door")}>the recipe card (the map, editable)</button>
                </p>
              </>
            ) : runningHere ? (
              <Zone>
                <Empty word="Running">
                  <p>The result lands here when the run finishes.</p>
                  <button onClick={() => setPlace("running")}>Watch it run</button>
                </Empty>
              </Zone>
            ) : runIsHere && run && (run.status === "failed" || run.status === "cancelled") ? (
              <>
                <Zone title={run.status === "cancelled" ? "You stopped this run" : "This run failed"} className="accent">
                  <pre className="error">{run.error ?? run.steps.find((s) => s.error)?.error ?? "no reason recorded"}</pre>
                  <button onClick={() => setPlace("setup")}>Back to Set up</button>
                </Zone>
                <TracePanel run={run} steps={steps} />
              </>
            ) : (
              <Zone>
                <Empty word="No result yet">
                  <p>Run this recipe and what it makes shows up here: notes, slides, tables, audio.</p>
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
            ? `${status.runCount} runs · ${status.tools.length} tools · recipe format v0`
            : "…"}
        </footer>
      </main>
    </>
  );
}

export default App;
