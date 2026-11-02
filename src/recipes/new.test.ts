import { describe, expect, it } from "vitest";
import { blankRecipe, slugify, uniqueId } from "./new";

describe("slugify", () => {
  it("makes a folder-safe id", () => {
    expect(slugify("My Weekly Report!")).toBe("my-weekly-report");
    expect(slugify("  Résumé — notes ")).toBe("resume-notes");
    expect(slugify("")).toBe("");
  });
});

describe("uniqueId", () => {
  it("adds a counter only when taken", () => {
    expect(uniqueId("report", [])).toBe("report");
    expect(uniqueId("report", ["report"])).toBe("report-2");
    expect(uniqueId("report", ["report", "report-2"])).toBe("report-3");
    expect(uniqueId("", [])).toBe("recipe");
  });
});

describe("blankRecipe", () => {
  it("reads a file and hands it back, with the icon only when given", () => {
    const d = blankRecipe("x", "X", "notes");
    expect(d.yaml.steps[0].tool).toBe("file.read");
    expect(d.yaml.outputs.text.from).toBe("${steps.read.output.text}");
    expect(d.yaml.icon).toBe("notes");
    expect(blankRecipe("y", "Y").yaml.icon).toBeUndefined();
  });
});
