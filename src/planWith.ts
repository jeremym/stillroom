import type { InstalledModel, PlanMeasurement, PlannerCandidate } from "./types";

/* Plan with: the words under the planner picker. The cost of a model is said before Draft it:
   its size, whether it fits in this Mac's memory, and what the last trial plan measured. The
   facts come from the Rust side (planner_preview); this file only puts them into sentences. */

/** Bytes as "29.9 GB" (one decimal) or "800 MB". */
export function gb(bytes: number): string {
  return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
}

/** Physical memory as Macs are sold: whole gigabytes, rounded ("36 GB"). */
export function memoryWord(bytes: number): string {
  return `${Math.round(bytes / 2 ** 30)} GB`;
}

/** A plan's time in words: "48 s", or "2 min 10 s" past a minute. */
export function fmtPlanTime(ms: number): string {
  const s = Math.round(ms / 1000);
  return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`;
}

/** The picker's label for one candidate: "glm-4.7-flash · 29.9B · local" (a cloud model says "cloud"). */
export function optionLabel(c: Pick<PlannerCandidate, "name" | "parameterSize" | "local">): string {
  return [c.name, c.parameterSize, c.local ? "local" : "cloud"].filter(Boolean).join(" · ");
}

export interface CostLine {
  /** "29.9 GB", or null when the provider does not say. */
  size: string | null;
  /** Fits or not, in words; amber when it does not fit or the model runs in the cloud. */
  fit: string;
  amber: boolean;
  /** What the last trial plan measured, or "not yet measured". */
  measured: string;
  /** The whole line, parts joined by " · ". */
  text: string;
}

/** What the last trial plan measured, in words. */
export function measuredWords(ms: number | null | undefined, clean: boolean | null | undefined): string {
  if (ms == null || clean == null) return "not yet measured";
  return clean ? `the last trial plan took ${fmtPlanTime(ms)} and checked clean` : `the last trial had problems after ${fmtPlanTime(ms)}`;
}

/** The cost line for one candidate on a Mac with `memoryBytes` of RAM (null when unknown). */
export function costLine(c: PlannerCandidate, memoryBytes: number | null | undefined): CostLine {
  const size = c.sizeBytes != null ? gb(c.sizeBytes) : null;
  let fit: string;
  let amber = false;
  if (!c.local) {
    fit = "runs in the cloud: what you type leaves this Mac";
    amber = true;
  } else if (c.fits) {
    fit = "fits in memory";
  } else {
    const where = memoryBytes != null ? ` on a ${memoryWord(memoryBytes)} Mac` : "";
    fit = `${size ?? "its size"}${where}: larger than this Mac holds comfortably; a smaller model plans in a fraction of the time`;
    amber = true;
  }
  const measured = measuredWords(c.measuredMs, c.measuredClean);
  // The size opens the line unless the fit sentence already carries it.
  const parts = [size && (c.fits || !c.local) ? size : null, fit, measured].filter((p): p is string => Boolean(p));
  return { size, fit, amber, measured, text: parts.join(" · ") };
}

/** The short tag on a model row in Settings: "plans in 48 s" or "plan had problems"; null when never measured. */
export function measuredTag(m: PlanMeasurement | null | undefined): { text: string; amber: boolean } | null {
  if (!m) return null;
  return m.clean ? { text: `plans in ${fmtPlanTime(m.ms)}`, amber: false } : { text: "plan had problems", amber: true };
}

/** True when a model could draft a recipe: it writes text, answers in a fixed shape (JSON), and runs here. */
export function canPlan(m: Pick<InstalledModel, "capabilities" | "local">): boolean {
  return m.local && m.capabilities.includes("text") && m.capabilities.includes("structured_output");
}
