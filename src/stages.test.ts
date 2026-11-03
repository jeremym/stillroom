import { describe, expect, it } from "vitest";
import { aboutMs, estimateWord, keepHeldWord, keepWord, outputsByStage, stageOfStep, stripFor } from "./stages";
import type { RecipeInfo, StepRecord } from "./types";

const recipe: RecipeInfo = {
  id: "r", name: "R", version: null, description: null, source: "builtin", path: "",
  inputs: { folder: { type: "directory" }, question: { type: "text" }, note: { type: "text", required: false } },
  modelSlots: {},
  steps: [
    { id: "list", kind: "tool", name: "List", slot: null, tool: "fs.list", dependsOn: [], reads: [] },
    { id: "answer", kind: "model", name: "Answer", slot: "writer", tool: null, dependsOn: ["list"], reads: [] },
    { id: "check", kind: "review", name: "Check", slot: null, tool: null, dependsOn: ["answer"], reads: [] },
  ] as unknown as RecipeInfo["steps"],
  outputs: { answer: { from: "${steps.answer.output}", type: "markdown" }, extra: { from: "${steps.list.output}", type: "json" } },
  stages: [
    { id: "docs", name: "Add documents", why: "The folder is all it reads.", inputs: ["folder"], steps: ["list"] },
    { id: "ask", name: "Ask", inputs: ["question", "note"], steps: ["answer"], outputs: ["answer"] },
    { id: "ok", name: "Your approval", steps: ["check"] },
  ],
};

function rec(stepId: string, status: StepRecord["status"], extra: Partial<StepRecord> = {}): StepRecord {
  return { runId: "x", stepId, kind: stepId === "check" ? "review" : "tool", name: null, position: 0, status, startedAt: null, finishedAt: null, durationMs: null, model: null, promptPath: null, promptTokens: null, completionTokens: null, inputs: null, output: null, error: null, ...extra };
}

describe("stripFor", () => {
  it("says which stages still need you before a run", () => {
    const s = stripFor(recipe, { inputs: { folder: "/x" } });
    expect(s.map((x) => x.status)).toEqual(["ready", "needs you", "your approval"]);
    expect(s[1].tone).toBe("amber");
    expect(stripFor(recipe)[0].status).toBe("you provide");
  });

  it("follows a run: running, done, waiting for you, failed", () => {
    const live = [rec("list", "succeeded"), rec("answer", "running")];
    const s = stripFor(recipe, { records: live, running: true });
    expect(s.map((x) => x.status)).toEqual(["done", "running 1 of 1", "waiting"]);
    const given = stripFor({ ...recipe, stages: [{ id: "in", name: "Give", inputs: ["folder"] }, ...recipe.stages.slice(1)] }, { records: live, running: true, inputs: { folder: "/x" } });
    expect(given[0].status).toBe("given");
    expect(s[1].progress).toBe(0);
    const paused = [rec("list", "succeeded", { reusedFrom: "old" }), rec("answer", "succeeded"), rec("check", "skipped")];
    const p = stripFor(recipe, { records: paused, runStatus: "needs_review" });
    expect(p.map((x) => x.status)).toEqual(["done · reused", "done", "waiting for you"]);
    expect(p[2].tone).toBe("amber");
    const bad = stripFor(recipe, { records: [rec("list", "failed")] });
    expect(bad[0]).toMatchObject({ status: "failed", tone: "bad" });
  });

  it("says what a kept stage holds", () => {
    const kept = { ...recipe, stages: [{ ...recipe.stages[0], keep: { refresh: "when_sources_change" as const } }, ...recipe.stages.slice(1)] };
    const k = (fresh: boolean | null, refresh: "always" | "when_sources_change" | "manual" = "when_sources_change") => ({ docs: { stage: "docs", records: 42, builtAt: "2026-10-31T12:00:00Z", fresh, refresh, last: null } });
    expect(stripFor(kept, { inputs: { folder: "/x" }, keeps: k(null) })[0].status).toBe("not built yet");
    expect(stripFor(kept, { inputs: { folder: "/x" }, keeps: k(true) })[0].status).toMatch(/^fresh · 42 records · built /);
    expect(stripFor(kept, { inputs: { folder: "/x" }, keeps: k(false) })[0]).toMatchObject({ status: "stale · will rebuild · 42 records", tone: "amber" });
    expect(stripFor(kept, { inputs: { folder: "/x" }, keeps: k(true, "always") })[0].status).toBe("runs every time · 42 records");
    // With the folder not yet chosen there is no verdict, only what the keep holds.
    expect(stripFor(kept, { inputs: {}, keeps: k(false) })[0]).toMatchObject({ status: "built Oct 31 · 42 records", tone: "green" });
    expect(stripFor(kept, { keeps: k(false) })[0].tone).toBe("green");
    // No Set up in sight: a stage with inputs only says it asks for them.
    expect(stripFor(recipe, {})[0]).toMatchObject({ status: "you provide", tone: "dim" });
    // A keep holding step outputs but no records (a kept spine) says when it was built, never "0 records".
    const spine = { ...k(true).docs, records: 0 };
    expect(keepWord(spine).status).toMatch(/^fresh · built /);
    expect(keepHeldWord(spine).status).toMatch(/^built /);
    expect(keepHeldWord({ ...spine, fresh: false }).tone).toBe("green");
    const skipped = [rec("list", "succeeded", { reusedFrom: "keep:docs" }), rec("answer", "running")];
    expect(stripFor(kept, { records: skipped, running: true })[0].status).toBe("ready · kept");
  });

  it("says nothing given for a stage whose optional input was left empty", () => {
    // Links or a folder, whichever you give: the links stage's loop skips with "nothing given for urls".
    const either: RecipeInfo = {
      ...recipe,
      inputs: { ...recipe.inputs, urls: { type: "urls", required: false }, folder: { type: "directory", required: false } },
      steps: [...recipe.steps, { id: "pages", kind: "map", name: "Read each link", slot: null, tool: null, dependsOn: [], reads: [] }] as unknown as RecipeInfo["steps"],
      stages: [
        { id: "links", name: "Your links", inputs: ["urls"], steps: ["pages"] },
        { id: "docs", name: "Your folder", inputs: ["folder"], steps: ["list"] },
        ...recipe.stages.slice(1),
      ],
    };
    const skippedFor = (stepId: string, input: string) => rec(stepId, "skipped", { skipReason: `nothing given for ${input}`, output: stepId === "pages" ? [] : null });
    // While running: the links stage was given nothing, the folder stage is running.
    const live = stripFor(either, { records: [skippedFor("pages", "urls"), rec("list", "running")], running: true, inputs: { folder: "/x" } });
    expect(live[0]).toMatchObject({ status: "nothing given", tone: "dim" });
    expect(live[1].status).toBe("running 1 of 1");
    // On Results: the folder was given nothing this time, the links ran.
    const done = stripFor(either, { records: [rec("pages", "succeeded"), skippedFor("list", "folder"), rec("answer", "succeeded"), rec("check", "succeeded")], runStatus: "succeeded" });
    expect(done.map((x) => x.status)).toEqual(["done", "nothing given", "done", "done"]);
    // A step skipped for another reason (a `when` that was not set) is still "done", and a loop over
    // a list that turned out empty is work that finished, not something withheld.
    const when = stripFor(either, { records: [rec("pages", "skipped", { skipReason: "${inputs.urls} was not set" }), rec("list", "skipped", { skipReason: "nothing to loop over", output: [] })] });
    expect(when.map((x) => x.status).slice(0, 2)).toEqual(["done", "done"]);
    // An inputs-only stage whose optional inputs were all left empty says the same during a run; needs you is never shown for an optional input.
    const inputsOnly = { ...either, stages: [{ id: "give", name: "Give", inputs: ["urls", "folder"] }, ...either.stages.slice(2)] };
    expect(stripFor(inputsOnly, { records: [rec("answer", "running")], running: true, inputs: { urls: [], folder: "" } })[0]).toMatchObject({ status: "nothing given", tone: "dim" });
    expect(stripFor(inputsOnly, { records: [rec("answer", "running")], running: true, inputs: { urls: ["https://a"], folder: "" } })[0].status).toBe("given");
    expect(stripFor(inputsOnly, { inputs: { urls: [], folder: "", question: "q" } })[0].status).not.toBe("needs you");
  });

  it("maps steps and outputs to stages", () => {
    expect(stageOfStep(recipe, "answer")?.id).toBe("ask");
    expect(outputsByStage(recipe)).toEqual([
      { stage: recipe.stages[1], names: ["answer"] },
      { stage: null, names: ["extra"] },
    ]);
  });
});

describe("estimate words", () => {
  it("says durations the way people plan by them", () => {
    expect(aboutMs(400)).toBe("under a second");
    expect(aboutMs(42_000)).toBe("42 s");
    expect(aboutMs(4 * 60_000 + 20_000)).toBe("4 min");
    expect(aboutMs(70 * 60_000)).toBe("1 h 10 min");
  });
  it("says whether a stage runs once or every run", () => {
    expect(estimateWord({ id: "a", basedOnRuns: 1, avgDurationMs: 240_000, avgTokens: 0, once: true })).toBe("about 4 min, once");
    expect(estimateWord({ id: "a", basedOnRuns: 3, avgDurationMs: 20_000, avgTokens: 0, once: false })).toBe("about 20 s, every run");
  });
});
