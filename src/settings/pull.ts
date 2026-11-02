/* Progress of a model download as block characters, the kit's way of showing progress. */
export function pullBar(completed: number | null | undefined, total: number | null | undefined, width = 24): { bar: string; pct: number | null; text: string } {
  if (!total || total <= 0 || completed == null) return { bar: "░".repeat(width), pct: null, text: "" };
  const pct = Math.max(0, Math.min(1, completed / total));
  const n = Math.round(pct * width);
  const gb = (b: number) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.round(b / 1e6)} MB`);
  return { bar: "▓".repeat(n) + "░".repeat(width - n), pct: Math.round(pct * 100), text: `${gb(completed)} of ${gb(total)}` };
}

/** The one model the welcome suggests when nothing installed can draft a recipe (plan): a planner-capable
 *  model sized to this Mac's memory. Under about 24 GB (a 16 GB Mac) it is qwen3:8b; from there up
 *  qwen3:14b. A first run never pulls a 30 B+ model. Unknown memory counts as small. */
export const PLANNER_STARTERS = { small: "qwen3:8b", large: "qwen3:14b" } as const;
export function suggestPlanner(memoryBytes: number | null | undefined): string {
  return memoryBytes != null && memoryBytes >= 24 * 2 ** 30 ? PLANNER_STARTERS.large : PLANNER_STARTERS.small;
}
