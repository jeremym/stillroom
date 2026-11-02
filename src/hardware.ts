import type { Hardware, ModelSpeed } from "./types";
import { memoryWord } from "./planWith";

/* This Mac: the facts Settings › Models says before the model table, so a model can be weighed
   against the machine before it is picked. The facts come from the Rust side (hardware); this
   file only puts them into words. */

/** The parts of the "This Mac" line, in order, each already worded; empty when nothing was read. */
export function thisMacParts(h: Hardware | null | undefined): string[] {
  if (!h) return [];
  const parts: string[] = [];
  if (h.chip) parts.push(h.chip);
  if (h.memoryBytes != null) parts.push(`${memoryWord(h.memoryBytes)} unified memory (shared by the CPU and GPU)`);
  if (h.gpuCores != null) parts.push(`${h.gpuCores} GPU cores`);
  if (h.cpuCores != null) parts.push(`${h.cpuCores} CPU cores`);
  if (h.modelCeilingBytes != null) parts.push(`room for models up to about ${memoryWord(h.modelCeilingBytes)}`);
  return parts;
}

/** One line: "Apple M3 Max · 96 GB unified memory (shared by the CPU and GPU) · 40 GPU cores · …", or null when nothing was read. */
export function thisMacLine(h: Hardware | null | undefined): string | null {
  const parts = thisMacParts(h);
  return parts.length ? parts.join(" · ") : null;
}

/** "66 tok/s" (whole numbers past 10, one decimal under), or "—" when the model has not answered yet. */
export function fmtSpeed(tps: number | null | undefined): string {
  if (tps == null || !Number.isFinite(tps) || tps <= 0) return "—";
  return `${tps >= 10 ? Math.round(tps) : tps.toFixed(1)} tok/s`;
}

/** The speed cell for one model from the table the engine sends. */
export function speedOf(speeds: Record<string, ModelSpeed> | undefined, modelId: string): string {
  return fmtSpeed(speeds?.[modelId]?.tokensPerSecond);
}
