import { invoke } from "@tauri-apps/api/core";
import type { CodexStatus, Draft, DraftResponse, JournalEntry, KeepRecord, KeepState, ModelsResponse, PlanOutcome, PlannerPreview, PromptState, TryResult, ModelUsage, PermissionEntry, RecipesResponse, ResolutionPreview, Run, RunEstimate, RunSummary, RuntimeStatus, SaveResponse, Settings, ToolInfo, WatchView } from "./types";

export const api = {
  runtimeStatus: () => invoke<RuntimeStatus>("runtime_status"),
  listModels: () => invoke<ModelsResponse>("list_models"),
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
  checkRecipeDraft: (draft: Draft) => invoke<string[]>("check_recipe_draft", { draft }),
  saveRecipeDraft: (draft: Draft) => invoke<SaveResponse>("save_recipe_draft", { draft }),
  /** The engine's own prompts (the planner) with their revisions. */
  listSystemPrompts: () => invoke<PromptState[]>("list_system_prompts"),
  readSystemPrompt: (name: string) => invoke<PromptState>("read_system_prompt", { name }),
  /** Save as a new revision and use it. Every window hears `prompts:changed`. */
  saveSystemPrompt: (name: string, text: string) => invoke<PromptState>("save_system_prompt", { name, text }),
  /** Use revision `n`; 0 is the original (reset). Nothing is deleted. */
  useSystemPromptVersion: (name: string, n: number) => invoke<PromptState>("use_system_prompt_version", { name, n }),
  /** Which model would plan right now, and which could. Nothing runs. */
  previewPlanner: (model?: string | null) => invoke<PlannerPreview>("preview_planner", { model: model ?? null }),
  /** Draft a recipe from a goal. Progress arrives as `plan:progress` events; Stop cancels. Nothing is saved. */
  planRecipe: (args: { goal: string; material?: string | null; id: string; name: string; icon?: string | null; model?: string | null }) =>
    invoke<PlanOutcome>("plan_recipe", { args }),
  /** Delete one of your recipes and the watches bound to it. Built-ins refuse. */
  deleteRecipe: (recipeId: string) => invoke<void>("delete_recipe", { recipeId }),
  revealRecipesFolder: () => invoke<void>("reveal_recipes_folder"),
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
  listRuns: (limit = 20) => invoke<RunSummary[]>("list_runs", { limit }),
  getRun: (id: string) => invoke<Run | null>("get_run", { id }),
  usage: () => invoke<ModelUsage[]>("usage"),
  estimate: (recipeId: string) => invoke<RunEstimate | null>("estimate", { recipeId }),
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
