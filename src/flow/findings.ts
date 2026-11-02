import type { Finding } from "../types";

/* Findings: what the checker says about a draft. The engine (Rust) decides what is a problem and what
   is a note; this file only reads its answer into one shape and sorts it for the screen. */

/** One finding from whatever the engine sent: a plain string (an older engine, or the planner's
 *  progress) is a problem; an object is taken as is, with a missing severity read as a problem. */
export function toFinding(f: string | Finding): Finding {
  if (typeof f === "string") return { text: f, severity: "problem" };
  return {
    text: typeof f.text === "string" ? f.text : String(f.text ?? ""),
    severity: f.severity === "note" ? "note" : "problem",
    fix: f.fix ?? null,
    fixLabel: f.fixLabel ?? null,
  };
}

export function toFindings(fs: (string | Finding)[] | null | undefined): Finding[] {
  return Array.isArray(fs) ? fs.map(toFinding) : [];
}

/** The problems: what must be fixed before saving. */
export function problemsOf(fs: Finding[]): Finding[] {
  return fs.filter((f) => f.severity === "problem");
}

/** The notes: worth a look, never in the way of saving. */
export function notesOf(fs: Finding[]): Finding[] {
  return fs.filter((f) => f.severity === "note");
}

/** The ids of every fix on offer, each once, in order. */
export function fixIds(fs: Finding[]): string[] {
  return [...new Set(fs.map((f) => f.fix).filter((id): id is string => typeof id === "string" && id.length > 0))];
}

/** Just the wording of the problems, for places that list them as plain text (the planner's progress). */
export function problemTexts(fs: (string | Finding)[] | null | undefined): string[] {
  return problemsOf(toFindings(fs)).map((f) => f.text);
}
