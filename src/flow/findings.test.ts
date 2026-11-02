import { describe, expect, it } from "vitest";
import { fixIds, notesOf, problemTexts, problemsOf, toFinding, toFindings } from "./findings";

describe("findings", () => {
  it("reads a plain string as a problem, so an older engine and the planner's progress still work", () => {
    expect(toFinding("steps[0] has no model")).toEqual({ text: "steps[0] has no model", severity: "problem" });
    expect(toFindings(["a", "b"])).toEqual([
      { text: "a", severity: "problem" },
      { text: "b", severity: "problem" },
    ]);
  });
  it("keeps a note with its fix and label, and fills the blanks", () => {
    const f = toFinding({ text: "This recipe uses `x`, which is now `y`.", severity: "note", fix: "x-to-y", fixLabel: "Use y" });
    expect(f).toEqual({ text: "This recipe uses `x`, which is now `y`.", severity: "note", fix: "x-to-y", fixLabel: "Use y" });
    expect(toFinding({ text: "no fix here", severity: "note" })).toEqual({ text: "no fix here", severity: "note", fix: null, fixLabel: null });
  });
  it("treats an unknown severity as a problem: the safe reading blocks Save", () => {
    expect(toFinding({ text: "odd", severity: "warning" as never }).severity).toBe("problem");
  });
  it("copes with nothing", () => {
    expect(toFindings(null)).toEqual([]);
    expect(toFindings(undefined)).toEqual([]);
  });
  it("splits problems from notes and lists each fix once", () => {
    const fs = toFindings([
      "broken",
      { text: "old a", severity: "note", fix: "a" },
      { text: "old a again", severity: "note", fix: "a" },
      { text: "old b", severity: "note", fix: "b" },
      { text: "just a thought", severity: "note" },
    ]);
    expect(problemsOf(fs).map((f) => f.text)).toEqual(["broken"]);
    expect(notesOf(fs)).toHaveLength(4);
    expect(fixIds(fs)).toEqual(["a", "b"]);
    expect(problemTexts(["x", { text: "y", severity: "note" }, { text: "z", severity: "problem" }])).toEqual(["x", "z"]);
  });
});
