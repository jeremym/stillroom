import { describe, expect, it } from "vitest";
import { buildGraph, describeRef, recordsFor, summarize } from "./graph";
import type { RecipeInfo, StepInfo, StepRecord } from "../types";

const step = (p: Partial<StepInfo> & { id: string; kind: string }): StepInfo => ({
  name: null, slot: null, tool: null, dependsOn: [], usesInputs: [], ...p,
});

/* A cut-down Inbox Triage: list → map(parse → classify → draft?, file) → table → review → outbox map. */
const recipe: RecipeInfo = {
  id: "t", name: "T", version: null, description: null, source: "builtin", path: "",
  inputs: {
    inbox: { type: "directory", label: "Inbox folder" },
    about: { type: "text", label: "About you", required: false },
    outbox: { type: "directory", label: "Outbox folder" },
  },
  modelSlots: { triager: { requires: ["text"] } },
  steps: [
    step({ id: "list", kind: "tool", tool: "fs.list", usesInputs: ["inbox"] }),
    step({
      id: "triage", kind: "map", name: "Handle each email", dependsOn: ["list"], usesInputs: ["about"], itemName: "email",
      steps: [
        step({ id: "parse", kind: "tool", tool: "email.parse" }),
        step({ id: "classify", kind: "model", name: "Sort it", slot: "triager", dependsOn: ["parse"], usesInputs: ["about"] }),
        step({ id: "draft", kind: "model", slot: "triager", dependsOn: ["classify", "parse"], when: "${steps.classify.output.needsReply}" }),
        step({ id: "file", kind: "agent", slot: "triager", dependsOn: ["classify", "parse"], allowedTools: ["fs.write"] }),
      ],
    }),
    step({ id: "table", kind: "tool", tool: "list.table", dependsOn: ["triage"] }),
    step({ id: "review", kind: "review", dependsOn: ["table"] }),
    step({ id: "outbox", kind: "map", dependsOn: ["review"], usesInputs: ["outbox"], steps: [step({ id: "write", kind: "tool" })] }),
  ],
  outputs: { triage: { from: "${steps.table.output.rows}", type: "table" } },
};

const rec = (p: Partial<StepRecord> & { stepId: string }): StepRecord => ({
  runId: "r", kind: "tool", name: null, position: 0, status: "succeeded", startedAt: null, finishedAt: null,
  durationMs: 5, model: null, promptPath: null, promptTokens: null, completionTokens: null, inputs: null, output: null, error: null, ...p,
});

const opts = { recipe, run: null, records: [], inProgress: false, modelFor: () => null, selectedId: null };

describe("buildGraph", () => {
  const g = buildGraph(opts);
  const node = (id: string) => g.nodes.find((n) => n.id === id)!;

  it("puts steps in rows by depth, inputs on the left and results on the right", () => {
    expect(node("step:list").position.y).toBeLessThan(node("step:triage").position.y);
    expect(node("step:triage").position.y).toBeLessThan(node("step:table").position.y);
    expect(node("step:review").position.y).toBeLessThan(node("step:outbox").position.y);
    expect(node("in:inbox").position.x).toBe(0);
    expect(node("out:triage").position.x).toBeGreaterThan(node("step:table").position.x);
    // An input sits beside the first row that reads it.
    expect(node("in:outbox").position.y).toBeGreaterThan(node("step:review").position.y);
    expect(g.width).toBeGreaterThan(0);
    expect(g.height).toBeGreaterThan(0);
  });

  it("nests map steps inside their group and lays inner steps left to right", () => {
    const parse = node("step:triage/parse");
    const classify = node("step:triage/classify");
    const draft = node("step:triage/draft");
    const file = node("step:triage/file");
    expect(parse.parentId).toBe("step:triage");
    expect(parse.position.x).toBeLessThan(classify.position.x);
    expect(classify.position.x).toBeLessThan(draft.position.x);
    expect(draft.position.x).toBe(file.position.x);
    expect(draft.position.y).toBeLessThan(file.position.y);
    expect(draft.data.condition).toBe("only if Sort it → needsReply");
    expect(node("step:triage").data.subtitle).toBe("4 inner steps per item");
  });

  it("draws edges with the right handles", () => {
    const edge = (id: string) => g.edges.find((e) => e.id === id)!;
    expect(edge("step:list->step:triage")).toMatchObject({ sourceHandle: "b", targetHandle: "t" });
    expect(edge("in:inbox->step:list")).toMatchObject({ sourceHandle: "r", targetHandle: "l" });
    expect(edge("step:table->out:triage")).toMatchObject({ sourceHandle: "r", targetHandle: "l" });
    expect(edge("step:triage/parse->step:triage/classify")).toMatchObject({ sourceHandle: "r", targetHandle: "l" });
    expect(g.edges.filter((e) => e.target === "step:triage/draft")).toHaveLength(2);
  });

  it("routes result lines from non-rightmost steps in a row under the row", () => {
    const fan: RecipeInfo = {
      ...recipe,
      steps: [
        step({ id: "a", kind: "tool" }),
        step({ id: "b", kind: "model", dependsOn: ["a"] }),
        step({ id: "c", kind: "model", dependsOn: ["a"] }),
      ],
      outputs: { bb: { from: "${steps.b.output}", type: "json" }, cc: { from: "${steps.c.output}", type: "json" } },
    };
    const g = buildGraph({ ...opts, recipe: fan });
    const bEdge = g.edges.find((e) => e.id === "step:b->out:bb")!;
    const cEdge = g.edges.find((e) => e.id === "step:c->out:cc")!;
    expect(bEdge).toMatchObject({ type: "bus", sourceHandle: "b" });
    expect((bEdge.data as { viaY: number }).viaY).toBeGreaterThan(g.nodes.find((n) => n.id === "step:b")!.position.y);
    expect(cEdge).toMatchObject({ type: "smoothstep", sourceHandle: "r" });
  });

  it("paints run state, aggregating per-item records for inner steps", () => {
    const records = [
      rec({ stepId: "list", reusedFrom: "old" }),
      rec({ stepId: "triage[0].classify", durationMs: 100 }),
      rec({ stepId: "triage[1].classify", durationMs: 100 }),
      rec({ stepId: "triage[0].draft" }),
      rec({ stepId: "triage[1].draft", status: "skipped", skipReason: "needsReply was not set" }),
      rec({ stepId: "triage", status: "running" }),
      rec({ stepId: "review", kind: "review", status: "skipped", skipReason: "2 items waiting for your decision" }),
      rec({ stepId: "outbox", status: "skipped", skipReason: "waiting for review" }),
    ];
    const painted = buildGraph({ ...opts, records, inProgress: true });
    const data = (id: string) => painted.nodes.find((n) => n.id === id)!.data;
    expect(data("step:list")).toMatchObject({ status: "reused", statusText: "reused" });
    expect(data("step:triage/classify")).toMatchObject({ status: "done", statusText: "2 done" });
    expect(data("step:triage/draft")).toMatchObject({ status: "done", statusText: "1 done · 1 skipped" });
    expect(data("step:triage/parse").status).toBe("waiting");
    expect(data("step:review").status).toBe("needs_you");
    expect(data("step:outbox").status).toBe("held");
    expect(painted.edges.find((e) => e.id === "step:list->step:triage")?.animated).toBe(true);
  });

  it("marks inner steps of a reused loop as reused", () => {
    const painted = buildGraph({ ...opts, records: [rec({ stepId: "triage", reusedFrom: "old" })] });
    expect(painted.nodes.find((n) => n.id === "step:triage/file")!.data.statusText).toBe("reused with the loop");
  });
});

describe("helpers", () => {
  it("describes references in words", () => {
    expect(describeRef("${inputs.about}", recipe)).toBe("About you");
    expect(describeRef("${steps.classify.output.needsReply}", recipe)).toBe("Sort it → needsReply");
    expect(describeRef("${email.path}", recipe, "email")).toBe("each email's path");
  });
  it("matches per-item records only for the right map", () => {
    const records = [rec({ stepId: "triage[0].parse" }), rec({ stepId: "other[0].parse" }), rec({ stepId: "parse" })];
    expect(recordsFor(records, "parse", "triage")).toHaveLength(1);
    expect(recordsFor(records, "parse")).toHaveLength(1);
  });
  it("summarizes a single record with its time", () => {
    expect(summarize([rec({ stepId: "x", durationMs: 1500 })], false)).toEqual({ status: "done", text: "done · 1.5 s" });
    expect(summarize([], true)).toEqual({ status: "waiting", text: "waiting" });
  });
});
