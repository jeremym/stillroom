import { describe, expect, it } from "vitest";
import { blankRecipe, copyRecipe, slugify, uniqueId } from "./new";

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

describe("copyRecipe", () => {
  it("renames, strips the built-in tag and author, keeps files", () => {
    const src = { yaml: { schemaVersion: "0", id: "a", name: "A", tags: ["built-in", "text"], author: { name: "S" }, steps: [], outputs: {} }, files: { "prompts/p.md": "hi" } };
    const out = copyRecipe(src, "b", "B", "mail", true);
    expect(out.yaml.id).toBe("b");
    expect(out.yaml.name).toBe("B");
    expect(out.yaml.icon).toBe("mail");
    expect(out.yaml.tags).toEqual(["text"]);
    expect(out.yaml.author).toBeUndefined();
    expect(out.files["prompts/p.md"]).toBe("hi");
    expect(src.yaml.id).toBe("a");
  });
});
