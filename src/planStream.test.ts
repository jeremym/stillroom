import { describe, expect, it } from "vitest";
import { EMPTY_STREAM, LINES_KEEP, TEXT_KEEP, callWord, modelPhaseWord, phaseWord, reducePlan, type PlanStreamState } from "./planStream";
import type { PlanEvent } from "./types";

const fold = (events: PlanEvent[], from: PlanStreamState = EMPTY_STREAM) => events.reduce((s, e) => reducePlan(s, e, 1000), from);

describe("reducePlan", () => {
  it("names the call and keeps the clock from a Phase, and a Delta adds text", () => {
    const s = fold([
      { kind: "phase", which: "strip", attempt: 1, of: 2, elapsedMs: 48_000, tokens: 612 },
      { kind: "delta", text: '{"stages": [', elapsedMs: 49_000, tokens: 620 },
    ]);
    expect(s.phase).toBe("strip");
    expect(s.attempt).toBe(1);
    expect(s.of).toBe(2);
    expect(s.elapsedMs).toBe(49_000);
    expect(s.tokens).toBe(620);
    expect(s.at).toBe(1000);
    expect(s.text).toBe('{"stages": [');
    expect(s.lines).toEqual(['{"stages": [']);
    expect(s.done).toBe(false);
  });
  it("keeps where the model call is until the next call starts, and says it in words until text flows", () => {
    let s = fold([{ kind: "phase", which: "strip", attempt: 1, of: 2 }, { kind: "model", phase: "loading", elapsedMs: 100, tokens: 0 }]);
    expect(s.modelPhase).toBe("loading");
    expect(modelPhaseWord(s.modelPhase)).toBe("loading the model into memory");
    s = fold([{ kind: "model", phase: "reading" }], s);
    expect(modelPhaseWord(s.modelPhase)).toBe("reading the goal");
    s = fold([{ kind: "model", phase: "writing" }, { kind: "delta", text: "{" }], s);
    expect(modelPhaseWord(s.modelPhase)).toBeNull();
    s = fold([{ kind: "phase", which: "steps", attempt: 1, of: 2 }], s);
    expect(s.modelPhase).toBeNull();
  });
  it("clears the text when a new Phase starts", () => {
    const s = fold([
      { kind: "phase", which: "strip", attempt: 1, of: 2 },
      { kind: "delta", text: "first call" },
      { kind: "phase", which: "steps", attempt: 2, of: 2 },
    ]);
    expect(s.phase).toBe("steps");
    expect(s.text).toBe("");
    expect(s.lines).toEqual([]);
  });
  it("keeps only the last dozen lines and the last few KB of text", () => {
    const many = Array.from({ length: 40 }, (_, i) => `line ${i}`).join("\n");
    const s = fold([{ kind: "phase", which: "steps", attempt: 1, of: 1 }, { kind: "delta", text: many }]);
    expect(s.lines).toHaveLength(LINES_KEEP);
    expect(s.lines[0]).toBe("line 28");
    expect(s.lines[LINES_KEEP - 1]).toBe("line 39");
    const big = fold([{ kind: "delta", text: "x".repeat(TEXT_KEEP + 500) }], s);
    expect(big.text).toHaveLength(TEXT_KEEP);
  });
  it("carries the problems into a fix round and counts them in the header", () => {
    const s = fold([
      { kind: "phase", which: "steps", attempt: 1, of: 1 },
      { kind: "delta", text: "draft" },
      { kind: "checking", attempt: 1 },
      { kind: "fixing", attempt: 1, problems: ["a step reads a missing input", { severity: "problem", text: "no result named out" }] },
      { kind: "phase", which: "fix", attempt: 2, of: 3 },
    ]);
    expect(s.text).toBe("");
    expect(s.problems).toEqual(["a step reads a missing input", "no result named out"]);
    expect(phaseWord(s)).toBe("Fixing 2 problems");
    expect(callWord(s)).toBe("try 2 of 3");
    // The next first draft forgets them.
    const next = fold([{ kind: "phase", which: "strip", attempt: 1, of: 2 }], s);
    expect(next.problems).toBeUndefined();
  });
  it("marks the stream done and keeps the text", () => {
    const s = fold([{ kind: "phase", which: "steps", attempt: 1, of: 1 }, { kind: "delta", text: "}" }, { kind: "done", elapsedMs: 130_000, tokens: 2000 }]);
    expect(s.done).toBe(true);
    expect(s.text).toBe("}");
    expect(s.elapsedMs).toBe(130_000);
  });
  it("still names the call from an older engine's events", () => {
    expect(phaseWord(fold([{ kind: "choosing" }]))).toBe("Choosing the stages");
    const t = fold([{ kind: "thinking", attempt: 1, of: 3 }]);
    expect(phaseWord(t)).toBe("Writing the custom stages");
    expect(callWord(t)).toBe("call 1 of 3");
    expect(phaseWord(fold([{ kind: "thinking", attempt: 2, of: 3 }]))).toBe("Fixing the draft");
    expect(phaseWord(EMPTY_STREAM)).toBeNull();
  });
});
