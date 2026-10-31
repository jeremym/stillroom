import { describe, expect, it } from "vitest";
import { outputsByStage, stageOfStep, stripFor } from "./stages";
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
    expect(stripFor(recipe)[0].status).toBe("needs you");
  });

  it("follows a run: running, done, waiting for you, failed", () => {
    const live = [rec("list", "succeeded"), rec("answer", "running")];
    const s = stripFor(recipe, { records: live, running: true });
    expect(s.map((x) => x.status)).toEqual(["done", "running 1 of 1", "waiting"]);
    expect(s[1].progress).toBe(0);
    const paused = [rec("list", "succeeded", { reusedFrom: "old" }), rec("answer", "succeeded"), rec("check", "skipped")];
    const p = stripFor(recipe, { records: paused, runStatus: "needs_review" });
    expect(p.map((x) => x.status)).toEqual(["done · reused", "done", "waiting for you"]);
    expect(p[2].tone).toBe("amber");
    const bad = stripFor(recipe, { records: [rec("list", "failed")] });
    expect(bad[0]).toMatchObject({ status: "failed", tone: "bad" });
  });

  it("maps steps and outputs to stages", () => {
    expect(stageOfStep(recipe, "answer")?.id).toBe("ask");
    expect(outputsByStage(recipe)).toEqual([
      { stage: recipe.stages[1], names: ["answer"] },
      { stage: null, names: ["extra"] },
    ]);
  });
});
