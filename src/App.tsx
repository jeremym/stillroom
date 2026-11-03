import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { Grant, ImportPreview, KeepState, ModelsResponse, ModelUsage, PlanEvent, RecipeInfo, RecipesResponse, Run, RunSummary, RuntimeStatus, Schedule, ScheduleRunEvent, Settings, StepRecord, WatchRunEvent, WatchView } from "./types";
import { RecipePanel } from "./components/RecipePanel";
import { TracePanel } from "./components/TracePanel";
import { Teletype } from "./components/Teletype";
import { ArtifactsPanel } from "./components/ArtifactsPanel";
import { ReviewPanel } from "./components/ReviewPanel";
import { HistoryPanel } from "./components/HistoryPanel";
import { WatchPanel } from "./components/WatchPanel";
import { SchedulePanel } from "./components/SchedulePanel";
import { UsagePanel } from "./components/UsagePanel";
import { RecipePage } from "./components/RecipePage";
import { NewRecipe, type NewRecipeRequest } from "./components/NewRecipe";
import { PlanNote, PlanStreamView } from "./components/PlanStreamView";
import { EMPTY_STREAM, phaseWord, reducePlan } from "./planStream";
import { AskAgain } from "./components/AskAgain";
import { FirstRun } from "./components/FirstRun";
import { ArchiveZone, CardMenu, EmptyShelf, RecipeCard } from "./components/Shelf";
import { CatalogPlace } from "./components/Catalog";
import { ImportPlace } from "./components/ImportPlace";
import { blankRecipe, slugify, uniqueId } from "./recipes/new";
import { Glyph, Logo, inputGlyph, isGlyphName } from "./brand";
import { isLocalModel, rememberProviders, shortModel, whereWord } from "./models";
import { Dial, Empty, Pill, Rail, Strip, Zone, applyLook, fmtElapsed, loadLook, useElapsed, type DialPlace, type Look } from "@overtone-foundry/app-kit";
import { outputsByStage, stripFor } from "./stages";
import { partitionShelf, shelfOf } from "./shelf";
import { problemTexts } from "./flow/findings";

/** The last part of a path: the recipe's folder, for a message that need not show the whole path. */
function folderName(path: string): string {
  const parts = path.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || path;
}
import "./App.css";

// React Flow is heavy; load it in its own chunk so the rest of the app paints first.
const FlowView = lazy(() => import("./flow/FlowView"));

/* Places. Library is home; the Catalog (browse, ⌘B) sits beside it. Opening a recipe puts the Dial on
   screen with that recipe's five places; the Dial turns by itself when a run starts, finishes, or
   needs you. Settings and New recipe sit outside the Dial, reached from the Rail or the menu. */
type Place = "first-run" | "library" | "browse" | "import" | "new" | "recipe" | "setup" | "running" | "results" | "door";
type RecipePlace = Exclude<Place, "first-run" | "library" | "browse" | "import" | "new">;
const PLACE_LABEL: Record<Place, string> = {
  "first-run": "First run", library: "Library", browse: "Catalog", import: "Get a recipe", new: "New recipe",
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
  const [schedules, setSchedules] = useState<Schedule[]>([]);
  const [place, setPlace] = useState<Place>("library");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [freshId, setFreshId] = useState<string | null>(null);
  // The planner at work: what it is doing, what its draft still got wrong, and a note for the card.
  const [planProgress, setPlanProgress] = useState<string | null>(null);
  const [planProblems, setPlanProblems] = useState<string[] | null>(null);
  const [planNote, setPlanNote] = useState<string | null>(null);
  // The planner's window: what the model is writing, folded from plan:progress; and everything it wrote, kept for the note.
  const [planStream, setPlanStream] = useState(EMPTY_STREAM);
  const [planTranscript, setPlanTranscript] = useState<string | null>(null);
  // The strip: which stage is selected, and what Set up holds so the strip can say who needs you.
  const [stageSel, setStageSel] = useState<string | null>(null);
  const [setupInputs, setSetupInputs] = useState<Record<string, unknown>>({});
  // What each kept stage of the open recipe holds, for the inputs typed so far.
  const [keeps, setKeeps] = useState<Record<string, KeepState>>({});
  const keepsFor = useCallback((id: string, inputs: Record<string, unknown>, pins: Record<string, string> = {}) => {
    api.keepStates(id, inputs, pins).then((list) => setKeeps(Object.fromEntries(list.map((k) => [k.stage, k])))).catch(() => setKeeps({}));
  }, []);
  const [helpOpen, setHelpOpen] = useState(false);
  const [confirmStop, setConfirmStop] = useState(false);
  // The first-run screen shows until a recipe has been drafted from it, or it was browsed past or skipped
  // once (firstRun absent means never shown). The hint then follows the first run to Results.
  const [firstRunHint, setFirstRunHint] = useState(false);
  const [stopping, setStopping] = useState(false);
  // A watch or a schedule that ran while another run was on screen: it stays out of the way, this line says it happened.
  const [backgroundNote, setBackgroundNote] = useState<{ text: string; runId: string | null } | null>(null);
  // A quiet line for something that went right and left no other trace (Saved to …).
  const [note, setNote] = useState<string | null>(null);
  // "Not now" on the update line is remembered for that version only; the next release shows again.
  const [dismissedVersion, setDismissedVersion] = useState<string | null>(() => { try { return localStorage.getItem("p1.updateDismissed"); } catch { return null; } });
  const updateDismissed = dismissedVersion !== null && dismissedVersion === settings.updates?.last?.latest;
  const dismissUpdate = (v: string) => { setDismissedVersion(v); try { localStorage.setItem("p1.updateDismissed", v); } catch { /* per-viewer */ } };
  useEffect(() => { if (place !== "library") setNote(null); }, [place]);
  // Get a recipe…: the pick, held in quarantine until Add or Discard. Leaving the place discards.
  const [importPreview, setImportPreview] = useState<ImportPreview | null>(null);
  const [importing, setImporting] = useState(false);
  const recipesRef = useRef<RecipesResponse | null>(null);
  // The run whose steps the screen follows. Steps and watch events from any other run are ignored.
  const liveRunRef = useRef<string | null>(null);
  // Watches and schedules whose run started in the background, by "watch:<id>" or "schedule:<id>", so
  // their finish is read the same way; and those runs' ids, so their steps never take the screen.
  const backgroundWatches = useRef(new Set<string>());
  const backgroundRuns = useRef(new Set<string>());
  const placeRef = useRef<Place>("library");
  placeRef.current = place;
  const runningRef = useRef(false);
  runningRef.current = running;

  const recipe = recipes?.recipes.find((r) => r.id === recipeId) ?? null;
  const shelf = useMemo(() => partitionShelf(recipes?.recipes ?? []), [recipes]);
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

  useEffect(() => { liveRunRef.current = run?.id ?? null; }, [run]);

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
    api.listModels().then((m) => { rememberProviders(m.providers); setModels(m); }).catch((e) => setError(String(e)));
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
  // An engine without schedules refuses the call; then there are none to show.
  const refreshSchedules = useCallback(() => {
    api.listSchedules().then(setSchedules).catch(() => setSchedules([]));
  }, []);

  const openRecipe = useCallback((id: string, where: RecipePlace = "recipe") => {
    setRecipeId(id);
    setFreshId(null);
    setPlace(where);
  }, []);

  // Nothing runs from the catalog without being shelved first, so the shelf tells the truth about
  // what has been used: opening Set up on a catalog recipe puts it on the shelf.
  const ensureShelved = useCallback(async (r: RecipeInfo) => {
    if (shelfOf(r) !== "catalog") return;
    await api.setShelf(r.id, "shelf");
    await refreshRecipes();
  }, [refreshRecipes]);
  useEffect(() => {
    if (place === "setup" && recipe) ensureShelved(recipe).catch((e) => setError(String(e)));
  }, [place, recipe, ensureShelved]);

  // Browse the catalog: its own place, beside the Library.
  const browseCatalog = useCallback(() => setPlace("browse"), []);

  // Get a recipe…: Rust opens the dialog and quarantines the pick; the preview is its own place.
  const importRef = useRef<ImportPreview | null>(null);
  importRef.current = importPreview;
  const getRecipe = useCallback(async (kind: "folder" | "zip" = "zip") => {
    setError(null);
    try {
      const p = await api.importRecipe(kind);
      if (!p) return;
      // A pick made while another preview was open replaces it; the earlier copy is thrown away.
      const before = importRef.current;
      if (before && before.token !== p.token) api.discardImport(before.token).catch(() => {});
      setImportPreview(p);
      setPlace("import");
    } catch (e) {
      setError(String(e));
    }
  }, []);
  // Leaving the import place without choosing throws the quarantined copy away.
  useEffect(() => {
    if (place === "import" || !importPreview) return;
    const { token } = importPreview;
    setImportPreview(null);
    api.discardImport(token).catch(() => {});
  }, [place, importPreview]);
  async function confirmImport(grants: Grant[]) {
    if (!importPreview) return;
    setImporting(true);
    setError(null);
    try {
      const info = await api.confirmImport(importPreview.token, grants);
      setImportPreview(null);
      await refreshRecipes();
      openRecipe(info.id);
    } catch (e) {
      setError(String(e));
    } finally {
      setImporting(false);
    }
  }
  function discardImport() {
    if (!importPreview) return;
    const { token } = importPreview;
    setImportPreview(null);
    api.discardImport(token).catch((e) => setError(String(e)));
    setPlace("library");
  }
  // Look at a catalog recipe: its page, read-only until it is shelved (RecipePage reads the shelf state).
  const lookAt = useCallback((id: string) => openRecipe(id), [openRecipe]);

  useEffect(() => {
    api.getSettings().then((s) => {
      setSettingsState(s);
      if (s.firstRun !== false) setPlace("first-run");
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
    refreshSchedules();
    const unlisten = listen<StepRecord>("run:step", (ev) => {
      // The first step of a fresh run names the run; a step from any other run is not ours to show.
      if (ev.payload.runId && backgroundRuns.current.has(ev.payload.runId)) return;
      if (liveRunRef.current && ev.payload.runId && ev.payload.runId !== liveRunRef.current) return;
      if (!liveRunRef.current && ev.payload.runId) liveRunRef.current = ev.payload.runId;
      setLiveSteps((prev) => {
        const next = prev.filter((s) => s.stepId !== ev.payload.stepId);
        next.push(ev.payload);
        return next.sort((a, b) => a.position - b.position);
      });
    });
    // A watch's or a schedule's run shows the same way a run from the Run button does, unless another
    // run is on screen: then it runs in the background and a quiet line says so.
    const onTriggeredRun = (key: string, byWord: string, refresh: () => void, w: { recipeId: string; phase: "started" | "finished" | "failed"; runId: string | null; status: string | null; error: string | null }) => {
      const recipeName = recipesRef.current?.recipes.find((r) => r.id === w.recipeId)?.name ?? w.recipeId;
      if (w.phase === "started") {
        const showing = runningRef.current || (liveRunRef.current && (placeRef.current === "running" || placeRef.current === "results"));
        if (showing) {
          backgroundWatches.current.add(key);
          if (w.runId) backgroundRuns.current.add(w.runId);
          refreshRuns();
          refresh();
          return;
        }
        backgroundWatches.current.delete(key);
        liveRunRef.current = w.runId ?? null;
        setRunning(true);
        setError(null);
        setRun(null);
        setLiveSteps([]);
        setRunRecipe(recipesRef.current?.recipes.find((r) => r.id === w.recipeId) ?? null);
        setRecipeId(w.recipeId);
        setPlace("running");
      } else if (backgroundWatches.current.has(key) || (w.runId && liveRunRef.current && w.runId !== liveRunRef.current)) {
        backgroundWatches.current.delete(key);
        if (w.runId) backgroundRuns.current.delete(w.runId);
        setBackgroundNote({ text: `${byWord} ran ${recipeName} in the background${w.status ? ` · ${STATUS_WORD[w.status] ?? w.status}` : w.error ? " · failed" : ""}.`, runId: w.runId });
        refreshRuns();
      } else {
        setRunning(false);
        if (w.error && !w.runId) setError(w.error);
        if (w.runId) openRun(w.runId).catch((e) => setError(String(e)));
        refreshRuns();
      }
      refresh();
    };
    const unlistenWatchRun = listen<WatchRunEvent>("watch:run", (ev) => onTriggeredRun(`watch:${ev.payload.watchId}`, "A watch", refreshWatches, ev.payload));
    const unlistenScheduleRun = listen<ScheduleRunEvent>("schedule:run", (ev) => onTriggeredRun(`schedule:${ev.payload.scheduleId}`, "A schedule", refreshSchedules, ev.payload));
    // The shelf changes in this window or another (Restore, Archive, Add to shelf).
    const unlistenShelf = listen("shelf:changed", () => { refreshRecipes().catch((e) => setError(String(e))); });
    // A recipe changed on the engine's side (the consent step wrote an answer, an import landed).
    const unlistenRecipes = listen("recipes:changed", () => { refreshRecipes().catch((e) => setError(String(e))); });
    const unlistenWatches = listen("watch:changed", () => refreshWatches());
    const unlistenSchedules = listen("schedule:changed", () => refreshSchedules());
    const unlistenPlan = listen<PlanEvent>("plan:progress", (ev) => {
      const e = ev.payload;
      setPlanStream((s) => reducePlan(s, e));
      if (e.kind === "phase") setPlanProgress(`${phaseWord({ phase: e.which, attempt: e.attempt, of: e.of }) ?? "Planning"}…`);
      else if (e.kind !== "delta" && e.kind !== "done") setPlanProgress(
        e.kind === "choosing" ? "Choosing the stages…"
          : e.kind === "thinking" ? (e.attempt === 1 ? "Thinking about the steps…" : `Fixing the draft (try ${e.attempt} of ${e.of})…`)
          : e.kind === "checking" ? "Checking the draft…"
          : `Found ${problemTexts(e.problems).length} problem${problemTexts(e.problems).length === 1 ? "" : "s"}; asking for a fix…`,
      );
    });
    // Settings change in the Settings window too; the model list changes when Ollama's address does.
    const unlistenSettings = listen<Settings>("settings:changed", (ev) => {
      const before = settingsRef.current.ollama?.baseUrl;
      setSettingsState(ev.payload);
      if (ev.payload.ollama?.baseUrl !== before) refreshModels();
    });
    return () => {
      unlisten.then((f) => f());
      unlistenWatchRun.then((f) => f());
      unlistenScheduleRun.then((f) => f());
      unlistenShelf.then((f) => f());
      unlistenRecipes.then((f) => f());
      unlistenWatches.then((f) => f());
      unlistenSchedules.then((f) => f());
      unlistenPlan.then((f) => f());
      unlistenSettings.then((f) => f());
    };
  }, [refreshModels, refreshRuns, refreshWatches, refreshSchedules, refreshRecipes, openRun]);

  // The native menu: every item sends its id here. Keyboard shortcuts live on the menu items.
  const onMenu = useCallback((id: string) => {
    const goRecipe = (p: RecipePlace) => { if (recipesRef.current?.recipes.length) setPlace((cur) => (isRecipePlace(cur) || recipeId ? p : cur)); };
    switch (id) {
      case "about": case "settings": api.openSettings().catch((e) => setError(String(e))); break;
      case "new-recipe": setCreateError(null); setPlace("new"); break;
      case "reveal-recipes": api.revealRecipesFolder().catch((e) => setError(String(e))); break;
      case "get-recipe": getRecipe("zip"); break;
      case "place-library": setPlace("library"); break;
      case "place-browse": setPlace("browse"); break;
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
  }, [recipeId, setLook, getRecipe]);
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
  // ⌘B (the Catalog) is bound here in every build: the native menu is built in Rust and has no item for
  // it yet; when one arrives with the same id, both paths land on the same place.
  useEffect(() => {
    const inTauri = "__TAURI_INTERNALS__" in window;
    const h = (e: KeyboardEvent) => {
      if (!e.metaKey || e.altKey || e.ctrlKey || e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      const map: Record<string, string> = inTauri
        ? { b: "place-browse" }
        : { ",": "settings", n: "new-recipe", b: "place-browse", "0": "place-library", "1": "place-recipe", "2": "place-setup", "3": "place-running", "4": "place-results", "5": "place-door", r: "run", "/": "shortcuts" };
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
    liveRunRef.current = null;
    setRunRecipe(recipe);
    setLiveSteps([]);
    setPlace("running");
    try {
      await ensureShelved(recipe);
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
      await ensureShelved(recipe);
      await api.addWatch(recipe.id, input, inputs, pins);
      refreshWatches();
      setPlace("recipe");
    } catch (e) {
      setError(String(e));
    }
  }

  /** Run every day at…: a schedule with Set up's inputs and pins. True once the engine has it. */
  async function scheduleRun(recipe: RecipeInfo, inputs: Record<string, unknown>, pins: Record<string, string>, time: string, days: string[]): Promise<boolean> {
    setError(null);
    try {
      await ensureShelved(recipe);
      await api.addSchedule(recipe.id, inputs, pins, time, days);
      refreshSchedules();
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  }

  async function editAndRerun(stepId: string, output: unknown) {
    if (!run) return;
    const priorId = run.id;
    setRunning(true);
    setError(null);
    setRun(null);
    liveRunRef.current = null;
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

  /** `firstRun`: drafted from the first-run screen; the note says so, the screen is done, and the hint follows the run. */
  async function createRecipe(req: NewRecipeRequest, firstRun = false) {
    setCreating(true);
    setCreateError(null);
    setPlanProblems(null);
    setPlanNote(null);
    setPlanStream(EMPTY_STREAM);
    setPlanTranscript(null);
    try {
      let draft;
      let note: string | null = null;
      if (req.mode === "goal" && req.goal) {
        setPlanProgress("Thinking about the steps…");
        const out = await api.planRecipe({ goal: req.goal, material: req.material ?? null, id: req.id, name: req.name, icon: req.icon, model: req.model ?? null });
        setPlanTranscript(out.transcript || null);
        const planProblemsLeft = problemTexts(out.problems);
        if (planProblemsLeft.length > 0) {
          setPlanProblems(planProblemsLeft);
          return;
        }
        draft = out.draft;
        const tries = out.attempts === 1 ? "first try" : `${out.attempts} tries`;
        note = firstRun
          ? "Your first recipe. Set up a run, or open the recipe card to see how it was made."
          : `Drafted by ${shortModel(out.modelId)} (${whereWord(out.modelId)}) on the ${tries} in ${fmtElapsed(Math.round(out.durationMs / 1000))}. Walk the steps and their prompts, then Save; nothing has run yet.`;
      } else if (req.mode === "copy" && req.sourceId) {
        // The engine makes the copy and writes where it came from (its provenance).
        const info = await api.duplicateRecipe(req.sourceId, req.id, req.name, req.icon ?? null);
        await refreshRecipes();
        setRecipeId(info.id);
        setFreshId(info.id);
        setPlanNote(null);
        setPlace("door");
        return;
      } else {
        draft = blankRecipe(req.id, req.name, req.icon);
      }
      const saved = await api.saveRecipeDraft(draft, true);
      await refreshRecipes();
      setRecipeId(saved.recipe.id);
      setFreshId(saved.recipe.id);
      setPlanNote(note);
      if (firstRun) {
        saveSettings({ firstRun: false });
        setFirstRunHint(true);
      }
      setPlace(req.mode === "goal" ? "recipe" : "door");
    } catch (e) {
      setCreateError(String(e));
    } finally {
      setCreating(false);
      setPlanProgress(null);
    }
  }

  async function duplicateRecipe(r: RecipeInfo) {
    setError(null);
    try {
      const taken = [...(recipesRef.current?.recipes.map((x) => x.id) ?? []), ...(recipesRef.current?.reserved ?? [])];
      const id = uniqueId(`${slugify(r.name)}-copy`, taken);
      const info = await api.duplicateRecipe(r.id, id, `${r.name} (copy)`);
      await refreshRecipes();
      openRecipe(info.id);
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
      refreshSchedules();
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

  useEffect(() => {
    if (!recipe || !recipe.stages.some((st) => st.keep)) { setKeeps({}); return; }
    const t = setTimeout(() => keepsFor(recipe.id, setupInputs, pins), 250);
    return () => clearTimeout(t);
  }, [recipe, setupInputs, pins, history, keepsFor]);

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
  const cardFacts = useMemo(() => ({ models: models?.models ?? [], history }), [models, history]);
  // A catalog recipe that is not shelved is looked at, not acted on, until Add to shelf.
  const looking = Boolean(recipe && shelfOf(recipe) === "catalog");
  const origin = recipe?.from ? recipes?.recipes.find((r) => r.id === recipe.from?.catalog) ?? null : null;
  const runsOfThis = run ? history.filter((h) => h.recipeId === run.recipeId) : [];

  return (
    <>
      <Rail
        left={
          <nav className="crumbs" aria-label="Where you are">
            <button className="home" onClick={() => setPlace("library")} title="Library: every recipe"><Logo size={28} /></button>
            {place === "library" && <><span className="sep">›</span><span className="here">Library</span></>}
            {(place === "browse" || place === "import") && <><span className="sep">›</span><button onClick={() => setPlace("library")} title="Library: your shelf">Library</button><span className="sep">›</span><span className="here">{PLACE_LABEL[place]}</span></>}
            {place === "new" && <><span className="sep">›</span><span className="here">{PLACE_LABEL[place]}</span></>}
            {isRecipePlace(place) && recipe && (
              <>
                {looking && <><span className="sep">›</span><button onClick={browseCatalog} title="The catalog (⌘B)">Catalog</button></>}
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
          <>
            <Dial places={places} current={place} onChange={(id) => setPlace(id as Place)} tone={needsYou && runIsHere ? "amber" : "green"} progress={runningHere ? progress : undefined} crumb={crumb} />
            <Strip
              label="Stages"
              stages={stripFor(recipe, {
                // Recipe and Set up read the recipe and what is typed; Running and Results read the run.
                inputs: place === "setup" ? setupInputs : place === "recipe" ? undefined : runIsHere ? run?.inputs : runningHere ? setupInputs : undefined,
                records: place === "recipe" || place === "setup" ? undefined : runIsHere || runningHere ? steps : undefined,
                running: runningHere,
                runStatus: runIsHere ? run?.status : null,
                keeps: place === "recipe" || place === "setup" ? keeps : undefined,
              })}
              current={stageSel}
              onSelect={(id) => {
                setStageSel(id);
                if (place === "setup") document.getElementById(`stage-${id}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
              }}
            />
          </>
        )}

        <div className="place">
          {helpOpen && (
            <Zone title="Help" sub="how Stillroom is laid out" aside={<button className="quiet" onClick={() => setHelpOpen(false)}>Close</button>}>
              <div className="facts">
                <span className="label">Library</span><span>Every recipe, every run, your watches and schedules. The logo at the top left always brings you back.</span>
                <span className="label">Recipe</span><span>Pick one and the Dial appears: <b>Recipe</b> (what it does, its runs) · <b>Set up</b> (inputs, models, Run) · <b>Running</b> (each step as it happens) · <b>Results</b> (what it made) · <b>Recipe card</b> (the recipe written out as a map you can edit).</span>
                <span className="label">Colours</span><span>Green is the machine. Paper is what it made for you. Amber means it needs you, or something would leave this Mac. Red is a failure.</span>
                <span className="label">Keys</span><span className="mono">⌘0 Library · ⌘B Catalog · ⌘1…⌘5 the Dial · ⌘N New recipe · ⌘R Run · ⌘, Settings · ⌘/ this help</span>
              </div>
            </Zone>
          )}

          {note && place === "library" && (
            <p className="quiet-line" role="status">
              <Glyph name="folder" /> {note}
              <button className="quiet" onClick={() => setNote(null)}>Dismiss</button>
            </p>
          )}

          {place === "library" && settings.updates?.last?.newer && settings.updates.last.latest && !updateDismissed && (
            <p className="quiet-line" role="status" data-testid="update-line">
              <Glyph name="help" /> Stillroom {settings.updates.last.latest} is out; you have {settings.updates.last.current}.
              {settings.updates.last.url && <button className="link" onClick={() => api.openLink(settings.updates!.last!.url!).catch((e) => setError(String(e)))}>Download from GitHub</button>}
              <button className="quiet" onClick={() => dismissUpdate(settings.updates!.last!.latest!)}>Not now</button>
            </p>
          )}

          {backgroundNote && (
            <p className="quiet-line" role="status">
              <Glyph name={backgroundNote.text.startsWith("A schedule") ? "calendar" : "watch"} /> {backgroundNote.text}
              {backgroundNote.runId && <button className="link" onClick={() => { const id = backgroundNote.runId!; setBackgroundNote(null); openRun(id).catch((e) => setError(String(e))); }}>See it</button>}
              <button className="quiet" onClick={() => setBackgroundNote(null)}>Dismiss</button>
            </p>
          )}

          {place === "first-run" && recipes && (
            <FirstRun recipes={recipes.recipes} reserved={recipes.reserved ?? []} busy={creating} error={createError} problems={planProblems}
              onCreate={(req) => createRecipe(req, true)} progress={planProgress}
              below={creating ? <PlanStreamView state={planStream} /> : planProblems && planTranscript ? <PlanNote note="The planner's draft was not saved; here is what it wrote." transcript={planTranscript} /> : undefined}
              onStop={() => api.stopRun("plan").catch((e) => setError(String(e)))}
              models={models} onRefreshModels={refreshModels} onOpenSettings={() => api.openSettings().catch((e) => setError(String(e)))}
              onSave={saveSettings} onBrowse={browseCatalog} onSkip={() => setPlace("library")} />
          )}

          {place === "library" && (
            <>
              <p className="tagline">Describe the goal. Get the result. See how it was made. <span className="muted">Pick a recipe to start, or make one.</span></p>
              <Zone title="Recipes" sub="reusable processes that produce a result"
                aside={<>
                  <span>{shelf.shelf.length} on the shelf</span>
                  <button className="quiet" onClick={browseCatalog} title="The catalog (⌘B)"><Glyph name="search" /> Browse the catalog</button>
                  <button className="quiet" onClick={() => getRecipe("zip")} title="Add a recipe from a zip someone gave you"><Glyph name="folder" /> Get a recipe from a zip…</button>
                  <button className="quiet" onClick={() => getRecipe("folder")} title="Add a recipe from a folder on this Mac"><Glyph name="folder" /> …or from a folder</button>
                  <button className="quiet" onClick={() => { setCreateError(null); setPlace("new"); }}><Glyph name="plus" /> New recipe</button>
                </>}>
                {shelf.shelf.length === 0 && recipes ? (
                  <EmptyShelf onBrowse={browseCatalog} onNew={() => { setCreateError(null); setPlace("new"); }} />
                ) : (
                  <div className="cards recipe-grid">
                    {shelf.shelf.map((r) => (
                      <RecipeCard key={r.id} recipe={r} facts={cardFacts} onOpen={(id) => openRecipe(id)}
                        foot={<CardMenu recipe={r} onSetup={(id) => openRecipe(id, "setup")} onDoor={(id) => openRecipe(id, "door")}
                          onDuplicate={duplicateRecipe} onDelete={deleteRecipe} onError={setError} onNote={setNote} onChanged={refreshRecipes} />} />
                    ))}
                  </div>
                )}
                {recipes?.errors.map((e) => (
                  <p key={e.path} className="load-error" title={e.path}>
                    <strong>{folderName(e.path)}</strong> could not be opened. {e.error}
                  </p>
                ))}
              </Zone>
              <ArchiveZone recipes={shelf.archive} facts={cardFacts} onOpen={(id) => openRecipe(id)} onError={setError} onChanged={refreshRecipes} />
              <WatchPanel watches={watches} onChanged={refreshWatches} onOpenRun={openRun} onError={(m) => setError(m)} />
              <SchedulePanel schedules={schedules} watchCount={watches.length} onChanged={refreshSchedules} onOpenRun={openRun} onError={(m) => setError(m)} />
              <HistoryPanel history={history} onOpen={openRun} />
              <UsagePanel usage={usage} />
            </>
          )}

          {place === "browse" && recipes && (
            <CatalogPlace recipes={recipes.recipes} facts={cardFacts} onLook={lookAt} onError={setError} onChanged={refreshRecipes} />
          )}

          {place === "import" && importPreview && (
            <ImportPlace preview={importPreview} models={models?.models ?? []} busy={importing} onConfirm={confirmImport} onDiscard={discardImport} onError={setError} />
          )}

          {place === "new" && recipes && (
            <NewRecipe recipes={recipes.recipes} reserved={recipes.reserved ?? []} busy={creating} error={createError} problems={planProblems} onCreate={createRecipe}
              progress={planStream.text && phaseWord(planStream) ? `${phaseWord(planStream)}…` : planProgress}
              below={creating ? <PlanStreamView state={planStream} /> : planProblems && planTranscript ? <PlanNote note="The planner's draft was not saved; here is what it wrote." transcript={planTranscript} /> : undefined}
              onCancel={() => setPlace(recipe ? "recipe" : "library")} onStop={() => api.stopRun("plan").catch((e) => setError(String(e)))} />
          )}

          {place === "recipe" && planNote && freshId === recipe?.id && <PlanNote note={planNote} transcript={planTranscript} />}
          {place === "recipe" && recipe && (
            <RecipePage recipe={recipe} models={models?.models ?? []} history={history} watches={watches} keeps={keeps}
              map={flowFor(recipe, false)}
              onSetup={() => setPlace("setup")} onDoor={() => setPlace("door")} onOpenRun={openRun}
              onDuplicate={() => duplicateRecipe(recipe)} onRename={(n) => renameRecipe(recipe, n)} onDelete={() => deleteRecipe(recipe)}
              onWatchesChanged={refreshWatches} onError={(m) => setError(m)}
              readOnly={looking} onAddToShelf={() => ensureShelved(recipe).catch((e) => setError(String(e)))} origin={origin} onLook={lookAt} />
          )}

          {place === "setup" && recipes && recipe && firstRunHint && (
            <p className="result-line hint-line"><Glyph name="sparkles" /> Choose what the recipe reads, then press <strong>Run</strong>. The Dial above follows the run to Results on its own.</p>
          )}
          {place === "setup" && recipes && recipe && (
            <RecipePanel recipes={shelf.shelf.some((r) => r.id === recipe.id) ? shelf.shelf : [recipe, ...shelf.shelf]} recipe={recipe} onSelect={(id) => openRecipe(id, "setup")} pins={pins} onPins={updatePins}
              models={models?.models ?? []} running={running} onRun={startRun} onWatch={watchFolder} onSchedule={scheduleRun} onSeeSchedules={() => setPlace("library")} onInputsChange={setSetupInputs}
              keeps={keeps} onRebuild={(stage) => api.rebuildKeep(recipe.id, stage).then(() => keepsFor(recipe.id, setupInputs, pins)).catch((e) => setError(String(e)))}
              onConsentChanged={() => refreshRecipes().catch((e) => setError(String(e)))} />
          )}

          {place === "running" && (
            runningHere || runIsHere ? (
              <>
                {runningHere && (
                  <div className="row stop-row">
                    {confirmStop
                      ? <><button className="danger" disabled={stopping} onClick={() => { setStopping(true); api.stopRun("run").catch((e) => setError(String(e))).finally(() => { setStopping(false); setConfirmStop(false); }); }}>{stopping ? "Stopping…" : "Really stop"}</button><button onClick={() => setConfirmStop(false)}>Keep running</button></>
                      : <button className="stop" onClick={() => setConfirmStop(true)}><Glyph name="stop" /> Stop</button>}
                    <span className="muted">Stop ends the run now: the step in progress is abandoned, and nothing already written is undone.</span>
                  </div>
                )}
                {flowFor(runRecipe ?? recipe, false)}
                <Zone title="As it happens" sub="each step typed as it starts and finishes (the trace)">
                  <Teletype records={runIsHere || runningHere ? steps : []} live={runningHere} stages={recipe?.stages} />
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
                          <span key={r.slot} className="fact"><Glyph name="ai" /> <span className="muted">{r.slot}:</span> <span className="mono">{r.modelId}</span> <span className={isLocalModel(r.modelId) ? "tag" : "tag amber"}>{whereWord(r.modelId)}</span></span>
                        ))}
                      {" "}<span className="muted">· {run.steps.filter((s) => !s.parent).length} steps{run.steps.some((s) => s.reusedFrom) ? `, ${run.steps.filter((s) => s.reusedFrom).length} reused from earlier runs` : ""}</span>
                    </span>
                    {(editedNames.length > 0 || run.trigger) && <>
                      <span className="label">Note</span>
                      <span className="muted">
                        {editedNames.length > 0 && <>Includes your {run.steps.some((s) => s.edited && s.kind === "review") ? "decision on" : "edit to"} <strong>{editedNames.join(", ")}</strong>. </>}
                        {run.trigger?.kind === "schedule" && <>Started by the schedule set for <strong>{run.trigger.time}</strong>.</>}
                        {run.trigger && run.trigger.kind !== "schedule" && <>Started by the folder watch{(run.trigger.changed ?? []).length > 0 ? <> after <strong>{run.trigger.changed.slice(0, 3).join(", ")}{run.trigger.changed.length > 3 ? ` and ${run.trigger.changed.length - 3} more` : ""}</strong> landed</> : " (Run now)"}.</>}
                      </span>
                    </>}
                  </div>
                  <p className="muted result-hint">Each result below is a tab. Edit one and only the steps after it run again.</p>
                </Zone>
                {run.status === "needs_review" && (
                  <ReviewPanel run={run} steps={runRecipe?.steps ?? []} busy={running} onDecide={editAndRerun} />
                )}
                <ArtifactsPanel run={run} outputs={runRecipe?.outputs ?? {}} steps={runRecipe?.steps ?? []} busy={running} onEdit={editAndRerun} groups={runRecipe ? outputsByStage(runRecipe).map((g) => ({ label: g.stage?.name ?? null, names: g.names })) : undefined} />
                {runRecipe && run.status === "succeeded" && (
                  <AskAgain recipe={runRecipe} run={run} running={running} onRun={(inputs) => startRun(runRecipe, inputs, pins, true)} />
                )}
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

          {place === "door" && planNote && freshId === recipe?.id && <PlanNote note={planNote} transcript={planTranscript} />}
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
