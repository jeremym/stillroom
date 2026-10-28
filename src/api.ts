import { invoke } from "@tauri-apps/api/core";
import type { CodexStatus, Draft, DraftResponse, ModelsResponse, TryResult, ModelUsage, PermissionEntry, RecipesResponse, ResolutionPreview, Run, RunEstimate, RunSummary, RuntimeStatus, SaveResponse, Settings, ToolInfo, WatchView } from "./types";

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
  /** Stop the run in flight. True if there was one. */
  stopRun: () => invoke<boolean>("stop_run"),
  /** Secrets live in the OS keychain. An empty value clears. The value never comes back. */
  setSecret: (name: string, value: string) => invoke<void>("set_secret", { name, value }),
  hasSecret: (name: string) => invoke<boolean>("has_secret", { name }),
  listRecipes: () => invoke<RecipesResponse>("list_recipes"),
  runRecipe: (recipeId: string, inputs: Record<string, unknown>, pins: Record<string, string>, reuse = true) =>
    invoke<Run>("run_recipe", { args: { recipeId, inputs, pins, reuse } }),
  /** Change propagation: rerun `runId` with `stepId`'s output replaced by the user's edit. */
  rerunWithEdit: (runId: string, stepId: string, output: unknown, reuse = true) =>
    invoke<Run>("rerun_with_edit", { args: { runId, stepId, output, reuse } }),
  clearStepCache: () => invoke<number>("clear_step_cache"),
  listTools: () => invoke<ToolInfo[]>("list_tools"),
  readRecipeDraft: (recipeId: string) => invoke<DraftResponse>("read_recipe_draft", { recipeId }),
  checkRecipeDraft: (draft: Draft) => invoke<string[]>("check_recipe_draft", { draft }),
  saveRecipeDraft: (draft: Draft) => invoke<SaveResponse>("save_recipe_draft", { draft }),
  /** Delete one of your recipes and the watches bound to it. Built-ins refuse. */
  deleteRecipe: (recipeId: string) => invoke<void>("delete_recipe", { recipeId }),
  revealRecipesFolder: () => invoke<void>("reveal_recipes_folder"),
  /** What macOS may ask for and where each stands. `probe` tests the folders, which is what makes macOS ask. */
  permissionStatus: (probe: boolean) => invoke<PermissionEntry[]>("permission_status", { probe }),
  openPrivacyPane: (pane: string) => invoke<void>("open_privacy_pane", { pane }),
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
  writeTextFile: (path: string, content: string) => invoke<void>("write_text_file", { path, content }),
  copyFile: (src: string, dst: string) => invoke<void>("copy_file", { src, dst }),
  writeBinaryFile: (path: string, bytes: Uint8Array) => {
    let bin = "";
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return invoke<void>("write_binary_file", { path, dataBase64: btoa(bin) });
  },
};

export function fmtMs(ms: number | null | undefined) {
  if (ms == null) return "";
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

export function fmtTokens(n: number) {
  return n >= 10_000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}
