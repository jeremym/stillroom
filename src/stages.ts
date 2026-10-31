import type { RecipeInfo, Run, Stage, StepRecord } from "./types";

/* The strip, as drawn: each stage of a recipe with one status word, from what is known right now:
   which inputs are filled (Set up), which steps have run (Running, Results), whether a review is
   waiting. Pure, so the places and the tests share one reading of the recipe. */

export type StageTone = "green" | "amber" | "dim" | "bad";

export interface StageStatus {
  id: string;
  label: string;
  why: string | null;
  status: string;
  tone: StageTone;
  /** 0 to 1 while this stage's steps are running. */
  progress?: number;
}

export interface StripContext {
  /** Values typed into Set up, to say which stage still needs you. */
  inputs?: Record<string, unknown>;
  /** Step records of the run shown (a finished run, or the live steps while running). */
  records?: StepRecord[];
  running?: boolean;
  runStatus?: Run["status"] | null;
}

function empty(v: unknown): boolean {
  return v == null || v === "" || (Array.isArray(v) && v.length === 0);
}

/** Top-level records only, by step id. */
function topRecords(records: StepRecord[]): Map<string, StepRecord> {
  const m = new Map<string, StepRecord>();
  for (const r of records) if (!r.parent) m.set(r.stepId, r);
  return m;
}

export function stageStatus(recipe: RecipeInfo, stage: Stage, ctx: StripContext): StageStatus {
  const base = { id: stage.id, label: stage.name, why: stage.why ?? null };
  const stepIds = stage.steps ?? [];
  const records = ctx.records ? topRecords(ctx.records) : null;

  // A run in progress or finished speaks first. A stage with only inputs was answered when the run started.
  if (records && records.size > 0 && stepIds.length === 0 && (stage.inputs ?? []).length > 0) {
    const missing = (stage.inputs ?? []).filter((name) => recipe.inputs[name]?.required !== false && empty(ctx.inputs?.[name]));
    return missing.length === 0 ? { ...base, status: "given", tone: "green" } : { ...base, status: "not given", tone: "dim" };
  }
  if (records && records.size > 0 && stepIds.length > 0) {
    const recs = stepIds.map((id) => records.get(id)).filter((r): r is StepRecord => Boolean(r));
    const failed = recs.find((r) => r.status === "failed");
    if (failed) return { ...base, status: "failed", tone: "bad" };
    const runningRec = recs.find((r) => r.status === "running");
    if (runningRec) {
      const done = recs.filter((r) => r.status === "succeeded" || r.status === "skipped").length;
      return { ...base, status: `running ${Math.min(done + 1, stepIds.length)} of ${stepIds.length}`, tone: "green", progress: done / stepIds.length };
    }
    const waiting = recs.find((r) => r.kind === "review" && r.status === "skipped" && ctx.runStatus === "needs_review");
    if (waiting) return { ...base, status: "waiting for you", tone: "amber" };
    const finished = recs.filter((r) => r.status === "succeeded" || r.status === "skipped");
    if (finished.length === stepIds.length) {
      const reused = recs.filter((r) => r.reusedFrom).length;
      return { ...base, status: reused === recs.length && reused > 0 ? "done · reused" : "done", tone: "green" };
    }
    if (ctx.running) return { ...base, status: "waiting", tone: "dim" };
    if (recs.length > 0) return { ...base, status: "not finished", tone: "dim" };
  }

  // Before a run: does this stage still need something from the user?
  const missing = (stage.inputs ?? []).filter((name) => {
    const spec = recipe.inputs[name];
    if (!spec || spec.required === false) return false;
    return empty(ctx.inputs?.[name]);
  });
  if (missing.length > 0) return { ...base, status: "needs you", tone: "amber" };
  if ((stage.inputs ?? []).length > 0 && ctx.inputs) return { ...base, status: "ready", tone: "green" };
  if ((stage.steps ?? []).some((id) => recipe.steps.find((s) => s.id === id)?.kind === "review")) return { ...base, status: "your approval", tone: "dim" };
  return { ...base, status: "—", tone: "dim" };
}

export function stripFor(recipe: RecipeInfo, ctx: StripContext = {}): StageStatus[] {
  return recipe.stages.map((s) => stageStatus(recipe, s, ctx));
}

/** Which stage a step belongs to, for teletype headings and the card. */
export function stageOfStep(recipe: RecipeInfo, stepId: string): Stage | undefined {
  return recipe.stages.find((s) => (s.steps ?? []).includes(stepId));
}

/** Outputs grouped by stage, in stage order; outputs no stage names come last under null. */
export function outputsByStage(recipe: RecipeInfo): { stage: Stage | null; names: string[] }[] {
  const seen = new Set<string>();
  const groups = recipe.stages
    .map((stage) => {
      const names = (stage.outputs ?? []).filter((n) => n in recipe.outputs);
      names.forEach((n) => seen.add(n));
      return { stage, names };
    })
    .filter((g) => g.names.length > 0);
  const rest = Object.keys(recipe.outputs).filter((n) => !seen.has(n));
  return rest.length ? [...groups, { stage: null, names: rest }] : groups;
}
