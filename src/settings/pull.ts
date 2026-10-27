/* Progress of a model download as block characters, the kit's way of showing progress. */
export function pullBar(completed: number | null | undefined, total: number | null | undefined, width = 24): { bar: string; pct: number | null; text: string } {
  if (!total || total <= 0 || completed == null) return { bar: "░".repeat(width), pct: null, text: "" };
  const pct = Math.max(0, Math.min(1, completed / total));
  const n = Math.round(pct * width);
  const gb = (b: number) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.round(b / 1e6)} MB`);
  return { bar: "▓".repeat(n) + "░".repeat(width - n), pct: Math.round(pct * 100), text: `${gb(completed)} of ${gb(total)}` };
}
