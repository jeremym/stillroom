import { describe, expect, it } from "vitest";
import { linesFor } from "./Teletype";
import type { StepRecord } from "../types";

const base: StepRecord = {
  runId: "r", stepId: "a", kind: "tool", name: "Read", position: 0, status: "succeeded",
  startedAt: "2026-10-27T14:02:11Z", finishedAt: "2026-10-27T14:02:12Z", durationMs: 1000, model: null, promptPath: null,
  promptTokens: null, completionTokens: null, inputs: null, output: null, error: null,
};

describe("linesFor", () => {
  it("prints a start and an end line for a finished step, in time order", () => {
    const l = linesFor([{ ...base, stepId: "b", name: "Write", kind: "model", model: "ollama:q", startedAt: "2026-10-27T14:02:13Z", finishedAt: "2026-10-27T14:02:20Z", promptTokens: 1200, completionTokens: 480 }, base]);
    expect(l.map((x) => x.mark)).toEqual(["▶", "✓", "▶", "✓"]);
    expect(l[2].text).toContain("Write · AI step · q · local");
    expect(l[3].text).toContain("1200 → 480 tokens");
  });
  it("says reused, failed, and needs you in their own words", () => {
    const l = linesFor([
      { ...base, reusedFrom: "old" },
      { ...base, stepId: "c", status: "failed", error: "boom" },
      { ...base, stepId: "d", kind: "review", status: "skipped", skipReason: "waiting for your decision" },
    ]);
    expect(l.find((x) => x.mark === "↻")?.tone).toBe("dim");
    expect(l.find((x) => x.mark === "✗")?.text).toContain("boom");
    expect(l.find((x) => x.mark === "■")?.tone).toBe("amber");
  });
  it("keeps a running step open with no end line", () => {
    const l = linesFor([{ ...base, status: "running", finishedAt: null, durationMs: null }]);
    expect(l).toHaveLength(1);
    expect(l[0].tone).toBe("bright");
  });
});
