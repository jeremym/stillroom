import { invoke } from "@tauri-apps/api/core";
import type { ModelsResponse, ModelUsage, RecipesResponse, Run, RunEstimate, RunSummary, RuntimeStatus } from "./types";

export const api = {
  runtimeStatus: () => invoke<RuntimeStatus>("runtime_status"),
  listModels: () => invoke<ModelsResponse>("list_models"),
  listRecipes: () => invoke<RecipesResponse>("list_recipes"),
  runRecipe: (recipeId: string, inputs: Record<string, unknown>, pins: Record<string, string>, reuse = true) =>
    invoke<Run>("run_recipe", { args: { recipeId, inputs, pins, reuse } }),
  clearStepCache: () => invoke<number>("clear_step_cache"),
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
