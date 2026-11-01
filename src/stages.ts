import type { KeepState, RecipeInfo, Run, Stage, StageEstimate, StepRecord } from "./types";

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
  /** Kept stages' state for the inputs at hand, by stage id. */
  keeps?: Record<string, KeepState>;
}

function keepParts(k: KeepState): { n: string; when: string } {
  // A keep that holds step outputs but no records (a kept spine, parsed pages) says when it was built, not "0 records".
  const n = k.records > 0 ? ` · ${k.records} record${k.records === 1 ? "" : "s"}` : "";
  const when = k.builtAt ? ` · built ${new Date(k.builtAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })}` : "";
  return { n, when };
}

/** The keep's state for the inputs at hand (Set up, the strip): fresh, stale, or not built. */
export function keepWord(k: KeepState): { status: string; tone: StageTone } {
  const { n, when } = keepParts(k);
  if (k.fresh == null) return { status: "not built yet", tone: "dim" };
  if (k.refresh === "always") return { status: `runs every time${n}`, tone: "green" };
  if (k.fresh) return { status: `fresh${n}${when}`, tone: "green" };
  return { status: `stale · will rebuild${n}`, tone: "amber" };
}

/** The keep's state when no inputs are known (the Recipe page): what it holds, with no verdict on freshness. */
export function keepHeldWord(k: KeepState): { status: string; tone: StageTone } {
  const { n, when } = keepParts(k);
  if (k.fresh == null) return { status: "not built yet", tone: "dim" };
  if (k.refresh === "always") return { status: `runs every time${n}`, tone: "green" };
  return { status: `${when.replace(/^ · /, "")}${n}`, tone: "green" };
}

/** A duration in words people plan by: "under a second", "40 s", "4 min", "1 h 10 min". */
export function aboutMs(ms: number): string {
  if (ms < 1000) return "under a second";
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  const rest = m - h * 60;
  return rest ? `${h} h ${rest} min` : `${h} h`;
}

/** "about 4 min, once" or "about 20 s, every run": what a stage costs and how often it runs. */
export function estimateWord(e: StageEstimate): string {
  return `about ${aboutMs(e.avgDurationMs)}, ${e.once ? "once" : "every run"}`;
}

/** Whether every required input of this stage and the stages before it has a value. */
export function inputsKnownUpTo(recipe: RecipeInfo, stage: Stage, inputs: Record<string, unknown> | undefined): boolean {
  if (!inputs) return false;
  const upTo = recipe.stages.slice(0, recipe.stages.indexOf(stage) + 1);
  return upTo.every((st) => (st.inputs ?? []).every((name) => recipe.inputs[name]?.required === false || !empty(inputs[name])));
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
      if (recs.length > 0 && recs.every((r) => r.reusedFrom?.startsWith("keep:"))) return { ...base, status: "ready · kept", tone: "green" };
      const reused = recs.filter((r) => r.reusedFrom).length;
      return { ...base, status: reused === recs.length && reused > 0 ? "done · reused" : "done", tone: "green" };
    }
    if (ctx.running) return { ...base, status: "waiting", tone: "dim" };
    if (recs.length > 0) return { ...base, status: "not finished", tone: "dim" };
  }

  // Before a run: a kept stage says what it holds; any stage says if it still needs you.
  const keep = ctx.keeps?.[stage.id];
  if (keep && stage.keep) {
    // Fresh or stale is a verdict about particular inputs; until the inputs this stage can see are
    // all given, say only what the keep holds.
    return { ...base, ...(inputsKnownUpTo(recipe, stage, ctx.inputs) ? keepWord(keep) : keepHeldWord(keep)) };
  }
  // With no Set up in sight (the Recipe page) a stage with inputs says only that it asks you for them.
  if (!ctx.inputs && (stage.inputs ?? []).some((name) => recipe.inputs[name]?.required !== false)) return { ...base, status: "you provide", tone: "dim" };
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
