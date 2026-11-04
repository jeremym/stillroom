import { invoke } from "@tauri-apps/api/core";
import { toFindings } from "./flow/findings";
import type { CodexStatus, Draft, DraftResponse, Finding, Grant, Hardware, ImportPreview, JournalEntry, KeepRecord, KeepState, ModelsResponse, PlanOutcome, PlannerPreview, PromptState, TryPlanningResult, TryResult, ModelUsage, PermissionEntry, PermissionRow, RecipeInfo, RecipesResponse, ResolutionPreview, Run, RunEstimate, RunSummary, RuntimeStatus, SaveResponse, Schedule, Settings, ShelfState, ToolInfo, UpdateCheck, WatchView } from "./types";

export const api = {
  runtimeStatus: () => invoke<RuntimeStatus>("runtime_status"),
  /** Ask GitHub's release feed whether a newer Stillroom exists. The one call the app makes beyond this Mac on its own; remembered under `updates.last`. */
  checkForUpdates: () => invoke<UpdateCheck>("check_for_updates"),
  listModels: () => invoke<ModelsResponse>("list_models"),
  /** This Mac: chip, memory, cores, and the room a model has. Read once by the engine; unknowns are null. */
  hardware: () => invoke<Hardware>("hardware"),
  /** The Codex CLI on this Mac: installed, signed in, its default model. */
  codexStatus: () => invoke<CodexStatus>("codex_status"),
  /** One tiny call to a model: did it answer, how fast. */
  tryModel: (modelId: string) => invoke<TryResult>("try_model", { modelId }),
  getSettings: () => invoke<Settings>("get_settings"),
  /** Merge top-level keys; a key set to null is removed. Every window hears `settings:changed`. */
  setSettings: (patch: Partial<Settings> | Record<string, unknown>) => invoke<Settings>("set_settings", { patch }),
  /** Download a model through Ollama; progress arrives as `model:pull` events. */
  pullModel: (name: string) => invoke<void>("pull_model", { name }),
  openSettings: () => invoke<void>("open_settings"),
  /** What each slot would get if the recipe ran now. Nothing runs. */
  previewResolution: (recipeId: string, pins: Record<string, string>) => invoke<ResolutionPreview>("preview_resolution", { args: { recipeId, pins } }),
  /** Stop a run in flight, the planner drafting a recipe, or both when `which` is left out. True if there was one. */
  stopRun: (which?: "run" | "plan") => invoke<boolean>("stop_run", { which: which ?? null }),
  /** Secrets live in the OS keychain. An empty value clears. The value never comes back. */
  setSecret: (name: string, value: string) => invoke<void>("set_secret", { name, value }),
  hasSecret: (name: string) => invoke<boolean>("has_secret", { name }),
  listRecipes: () => invoke<RecipesResponse>("list_recipes"),
  runRecipe: (recipeId: string, inputs: Record<string, unknown>, pins: Record<string, string>, reuse = true, rebuild: string[] = []) =>
    invoke<Run>("run_recipe", { args: { recipeId, inputs, pins, reuse, rebuild } }),
  /** What each kept stage holds and whether it is fresh for these inputs. */
  keepStates: (recipeId: string, inputs: Record<string, unknown>, pins: Record<string, string> = {}) => invoke<KeepState[]>("keep_states", { args: { recipeId, inputs, pins } }),
  /** Empty a stage's keep now; the next run builds it again. */
  rebuildKeep: (recipeId: string, stage: string) => invoke<void>("rebuild_keep", { recipeId, stage }),
  keepJournal: (recipeId: string, stage: string, limit = 50) => invoke<JournalEntry[]>("keep_journal", { recipeId, stage, limit }),
  keepRecords: (recipeId: string, stage: string, limit = 200) => invoke<KeepRecord[]>("keep_records", { recipeId, stage, limit }),
  /** Change propagation: rerun `runId` with `stepId`'s output replaced by the user's edit. */
  rerunWithEdit: (runId: string, stepId: string, output: unknown, reuse = true) =>
    invoke<Run>("rerun_with_edit", { args: { runId, stepId, output, reuse } }),
  clearStepCache: () => invoke<number>("clear_step_cache"),
  listTools: () => invoke<ToolInfo[]>("list_tools"),
  readRecipeDraft: (recipeId: string) => invoke<DraftResponse>("read_recipe_draft", { recipeId }),
  /** What is wrong (problems) or worth a look (notes) in a draft. An older engine answers with plain
   *  strings; those are read as problems. */
  checkRecipeDraft: (draft: Draft): Promise<Finding[]> => invoke<(string | Finding)[]>("check_recipe_draft", { draft }).then(toFindings),
  /** Apply the named fixes (the `fix` ids from findings) and get the rewritten draft back. Nothing is saved. */
  fixRecipeDraft: (draft: Draft, fixes: string[]) => invoke<Draft>("fix_recipe_draft", { draft, fixes }),
  /** `expectNew` says this makes a recipe rather than editing one: an id already on disk is refused, never written over. */
  saveRecipeDraft: (draft: Draft, expectNew = false) => invoke<SaveResponse>("save_recipe_draft", { draft, expectNew }),
  /** The engine's own prompts (the planner) with their revisions. */
  listSystemPrompts: () => invoke<PromptState[]>("list_system_prompts"),
  readSystemPrompt: (name: string) => invoke<PromptState>("read_system_prompt", { name }),
  /** Save as a new revision and use it. Every window hears `prompts:changed`. */
  saveSystemPrompt: (name: string, text: string) => invoke<PromptState>("save_system_prompt", { name, text }),
  /** Use revision `n`; 0 is the original (reset). Nothing is deleted. */
  useSystemPromptVersion: (name: string, n: number) => invoke<PromptState>("use_system_prompt_version", { name, n }),
  /** Which model would plan right now, and which could. Nothing runs. */
  previewPlanner: (model?: string | null) => invoke<PlannerPreview>("preview_planner", { model: model ?? null }),
  /** Try planning: the planner's first call on a fixed sample goal with this model, timed and checked;
   *  the result is remembered under planner.measured so Auto can prefer it. Up to five minutes; `stopRun("plan")` cancels. */
  tryPlanning: (modelId: string) => invoke<TryPlanningResult>("try_planning", { modelId }),
  /** Draft a recipe from a goal. Progress arrives as `plan:progress` events; Stop cancels. Nothing is saved. */
  planRecipe: (args: { goal: string; material?: string | null; id: string; name: string; icon?: string | null; model?: string | null }) =>
    invoke<PlanOutcome>("plan_recipe", { args }),
  /** Delete one of your recipes and the watches bound to it. Built-ins refuse. */
  deleteRecipe: (recipeId: string) => invoke<void>("delete_recipe", { recipeId }),
  /** A copy of a recipe under a new id and name, in your recipes folder. The engine writes where it
   *  came from (its provenance) so the Library can say when the catalog has a newer version. */
  duplicateRecipe: (recipeId: string, newId: string, name: string, icon?: string | null) =>
    invoke<RecipeInfo>("duplicate_recipe", { recipeId, newId, name, icon: icon ?? null }),
  /** The consent step: allow the literal permission entries the user ticked. Every window hears `recipes:changed`. */
  grantPermissions: (recipeId: string, entries: Grant[]) => invoke<RecipeInfo>("grant_permissions", { recipeId, entries }),
  /** Take back every permission granted to a recipe; it will not run until ticked again. */
  revokePermissions: (recipeId: string) => invoke<RecipeInfo>("revoke_permissions", { recipeId }),
  /** The saved example result a catalog recipe ships, as a run record for the ordinary viewers. */
  exampleResult: (recipeId: string) => invoke<Run>("example_result", { recipeId }),
  /** Get a recipe from a folder or a zip. Rust opens the dialog, copies the pick into quarantine, and
   *  checks it; null when the user cancelled. Nothing joins your recipes until `confirmImport`. */
  importRecipe: (kind: "folder" | "zip" = "zip"): Promise<ImportPreview | null> =>
    invoke<ImportPreview | null>("import_recipe", { kind }).then((p) => (p ? { ...p, findings: toFindings(p.findings as (string | Finding)[]) } : null)),
  /** Move a quarantined recipe into your recipes folder with the permissions ticked. */
  confirmImport: (token: string, grants: Grant[]) => invoke<RecipeInfo>("confirm_import", { token, grants }),
  /** Throw a quarantined recipe away. */
  discardImport: (token: string) => invoke<void>("discard_import", { token }),
  /** Save a copy of a recipe as a zip where the user chooses (dialog in Rust). The path written, or null when cancelled. */
  exportRecipe: (recipeId: string) => invoke<string | null>("export_recipe", { recipeId }),
  revealRecipesFolder: () => invoke<void>("reveal_recipes_folder"),
  /** Show the planner's journal (one Markdown file per plan) in the Finder. */
  revealPlansFolder: () => invoke<void>("reveal_plans_folder"),
  /** Show the sample folders (the inboxes a sample recipe reads) in the Finder. */
  revealSampleFolders: () => invoke<void>("reveal_sample_folders"),
  /** Put a recipe on the shelf, in the archive, or back in the catalog. Every window hears `shelf:changed`. */
  setShelf: (recipeId: string, state: ShelfState) => invoke<void>("set_shelf", { recipeId, state }),
  /** What macOS may ask for and where each stands. `probe` tests the folders, which is what makes macOS ask. */
  permissionStatus: (probe: boolean) => invoke<PermissionEntry[]>("permission_status", { probe }),
  openPrivacyPane: (pane: string) => invoke<void>("open_privacy_pane", { pane }),
  /** Open a web or mail link outside the app. Rust refuses every other kind of address. */
  openLink: (url: string) => invoke<void>("open_link", { url }),
  listWatches: () => invoke<WatchView[]>("list_watches"),
  /** Watch `input`'s folder: the recipe runs with these inputs whenever something lands there. */
  addWatch: (recipeId: string, input: string, inputs: Record<string, unknown>, pins: Record<string, string>) =>
    invoke<WatchView>("add_watch", { args: { recipeId, input, inputs, pins } }),
  setWatchEnabled: (id: string, enabled: boolean) => invoke<WatchView>("set_watch_enabled", { id, enabled }),
  removeWatch: (id: string) => invoke<void>("remove_watch", { id }),
  fireWatch: (id: string) => invoke<void>("fire_watch", { id }),
  /** Schedules: a recipe that runs at a time of day while Stillroom is open. An engine without them refuses the call. */
  listSchedules: () => invoke<Schedule[]>("list_schedules"),
  /** `days` are "mon".."sun"; empty means every day. Refused like a run while the consent step is unanswered. */
  addSchedule: (recipeId: string, inputs: Record<string, unknown>, pins: Record<string, string>, time: string, days: string[]) =>
    invoke<Schedule>("add_schedule", { args: { recipeId, inputs, pins, time, days } }),
  setScheduleEnabled: (id: string, enabled: boolean) => invoke<Schedule>("set_schedule_enabled", { id, enabled }),
  deleteSchedule: (id: string) => invoke<void>("delete_schedule", { id }),
  fireSchedule: (id: string) => invoke<Run>("fire_schedule", { id }),
  listRuns: (limit = 20) => invoke<RunSummary[]>("list_runs", { limit }),
  getRun: (id: string) => invoke<Run | null>("get_run", { id }),
  usage: () => invoke<ModelUsage[]>("usage"),
  /** What a run will take: from earlier runs, else a guess from the models these pins and inputs would use. */
  estimate: (recipeId: string, pins?: Record<string, string>, inputs?: Record<string, unknown>) => invoke<RunEstimate | null>("estimate", { recipeId, pins, inputs }),
  /** What a run with these inputs would touch, one row per permission entry, every chosen folder tried. */
  probePermissions: (recipeId: string, inputs: Record<string, unknown>) => invoke<PermissionRow[]>("probe_permissions", { recipeId, inputs }),
  /** Save text where the user chooses. The save dialog runs on the Rust side; the webview never names a path.
   *  Resolves to the path written, or null when the user cancelled. */
  saveTextAs: (name: string, ext: string, content: string) => invoke<string | null>("save_text_as", { name, ext, content }),
  /** Save bytes where the user chooses (dialog in Rust). Null when cancelled. */
  saveBinaryAs: (name: string, ext: string, bytes: Uint8Array) => {
    let bin = "";
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return invoke<string | null>("save_binary_as", { name, ext, dataBase64: btoa(bin) });
  },
  /** Copy a file a run produced (audio, say) to where the user chooses. Rust checks the file belongs to the run. Null when cancelled. */
  saveArtifactAs: (runId: string, artifactPath: string, name: string, ext: string) =>
    invoke<string | null>("save_artifact_as", { runId, artifactPath, name, ext }),
};

export function fmtMs(ms: number | null | undefined) {
  if (ms == null) return "";
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

export function fmtTokens(n: number) {
  return n >= 10_000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}
