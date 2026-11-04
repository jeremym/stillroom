import type { CallPhase, PlanEvent, PlanPhase } from "./types";
import { problemTexts } from "./flow/findings";

/* The planner's window, folded from `plan:progress` events: which call is in flight, how far along,
   and the tail of what the model has written. A new call (a Phase event) clears the text; a fix round
   keeps the problems it was asked to fix so the header can count them. Pure, so it is testable. */

export const TEXT_KEEP = 4096;
export const LINES_KEEP = 12;

export interface PlanStreamState {
  phase: PlanPhase | null;
  attempt: number;
  of: number;
  /** The engine's clock at the last event; the view adds the time since. */
  elapsedMs: number;
  tokens: number;
  /** When the last event arrived (Date.now()), so the elapsed time keeps moving between events. */
  at: number;
  /** The tail of the current call's text, the last TEXT_KEEP characters. */
  text: string;
  /** The last LINES_KEEP lines of `text`. */
  lines: string[];
  /** What the checker found, when the current call is a fix round. */
  problems?: string[];
  /** Where the model call is: loading the model, reading the prompt, writing. */
  modelPhase?: CallPhase | null;
  done: boolean;
}

export const EMPTY_STREAM: PlanStreamState = { phase: null, attempt: 0, of: 0, elapsedMs: 0, tokens: 0, at: 0, text: "", lines: [], modelPhase: null, done: false };

export function tailLines(text: string, keep = LINES_KEEP): string[] {
  if (!text) return [];
  return text.split("\n").slice(-keep);
}

/** Fold one event into the window's state. `now` is injected so tests are deterministic. */
export function reducePlan(s: PlanStreamState, e: PlanEvent, now = Date.now()): PlanStreamState {
  const clock = { elapsedMs: e.elapsedMs ?? s.elapsedMs, tokens: e.tokens ?? s.tokens, at: now };
  switch (e.kind) {
    case "phase":
      return { ...s, ...clock, phase: e.which, attempt: e.attempt, of: e.of, text: "", lines: [], done: false, modelPhase: null, problems: e.which === "fix" ? s.problems : undefined };
    case "model":
      return { ...s, ...clock, modelPhase: e.phase };
    case "delta": {
      const text = (s.text + e.text).slice(-TEXT_KEEP);
      return { ...s, ...clock, text, lines: tailLines(text) };
    }
    case "done":
      return { ...s, ...clock, done: true };
    case "fixing":
      return { ...s, ...clock, problems: problemTexts(e.problems) };
    // Older engines send only these; they still name the call so the header has something to say.
    case "choosing":
      return { ...s, ...clock, phase: "strip", attempt: 1, of: 1 };
    case "thinking":
      return { ...s, ...clock, phase: e.attempt > 1 ? "fix" : "steps", attempt: e.attempt, of: e.of };
    case "checking":
      return { ...s, ...clock };
  }
}

/** The header's first words: what the planner is doing, in plain terms. */
export function phaseWord(s: Pick<PlanStreamState, "phase" | "attempt" | "of" | "problems">): string | null {
  switch (s.phase) {
    case "strip": return "Choosing the stages";
    case "steps": return "Writing the custom stages";
    case "fix": {
      const n = s.problems?.length ?? 0;
      return n > 0 ? `Fixing ${n} problem${n === 1 ? "" : "s"}` : "Fixing the draft";
    }
    default: return null;
  }
}

/** Where the model call is, in plain words; nothing once text is flowing, since the text says so. */
export function modelPhaseWord(phase: CallPhase | null | undefined, subject = "the goal"): string | null {
  switch (phase) {
    case "loading": return "loading the model into memory";
    case "reading": return `reading ${subject}`;
    default: return null;
  }
}

/** The call counter after the phase word: "call 1 of 2" for a first draft, "try 2 of 3" for a fix round. */
export function callWord(s: Pick<PlanStreamState, "phase" | "attempt" | "of">): string | null {
  if (!s.phase || s.of === 0) return null;
  return s.phase === "fix" ? `try ${s.attempt} of ${s.of}` : `call ${s.attempt} of ${s.of}`;
}
