import { describe, expect, it } from "vitest";
import { catalogRecipes, catalogRows, catalogTags, filterCatalog, needsLine, runsOnWhatIHave, sourceWord } from "./catalog";
import type { InstalledModel, RecipeInfo } from "./types";

function recipe(id: string, extra: Partial<RecipeInfo> = {}): RecipeInfo {
  return { id, name: `Recipe ${id}`, version: "0.1.0", description: null, source: "builtin", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [], ...extra };
}
function model(id: string, capabilities: InstalledModel["capabilities"]): InstalledModel {
  return { id, provider: "ollama", name: id, capabilities, contextWindow: null, parameterSize: null, local: true };
}

const docs = recipe("ask", { name: "Ask Your Documents", description: "Index a folder once, then ask it anything.", tags: ["built-in", "documents", "keep"],
  inputs: { folder: { type: "directory", label: "Folder" }, question: { type: "string", label: "Question" } },
  modelSlots: { writer: { requires: ["text"] }, embedder: { requires: ["embedding"] } },
  stages: [{ id: "index", name: "Index", why: "so later questions are cheap" }] });
const brief = recipe("brief", { name: "Research Brief", description: "Read links, quote them.", tags: ["built-in", "research", "web"],
  inputs: { links: { type: "urls", label: "Links" } }, modelSlots: { writer: { requires: ["text"] } } });
const sample = recipe("inbox", { name: "Inbox Triage", source: "sample", tags: ["sample", "email"] });
const community = recipe("notes", { name: "Meeting Notes", source: "community", tags: ["documents"], author: { name: "Someone" } });
const mine = recipe("mine", { source: "user", tags: ["documents"] });
const all = [docs, brief, sample, community, mine];

describe("catalogRecipes and rows", () => {
  it("leaves your own recipes out and sorts the rest into three rows", () => {
    expect(catalogRecipes(all).map((r) => r.id)).toEqual(["ask", "brief", "inbox", "notes"]);
    const rows = catalogRows(catalogRecipes(all));
    expect(rows.builtin.map((r) => r.id)).toEqual(["ask", "brief"]);
    expect(rows.community.map((r) => r.id)).toEqual(["notes"]);
    expect(rows.sample.map((r) => r.id)).toEqual(["inbox"]);
  });

  it("reads an unknown source as built-in", () => {
    expect(sourceWord("builtin")).toBe("built-in");
    expect(sourceWord("future" as never)).toBe("built-in");
    expect(sourceWord("user")).toBe("yours");
    expect(catalogRows([recipe("x", { source: "future" as never })]).builtin).toHaveLength(1);
  });
});

describe("catalogTags", () => {
  it("lists the distinct tags once, in order, without the row words", () => {
    expect(catalogTags(catalogRecipes(all))).toEqual(["documents", "keep", "research", "web", "email"]);
  });
  it("treats missing tags as none", () => {
    expect(catalogTags([recipe("bare")])).toEqual([]);
  });
});

describe("filterCatalog", () => {
  const text = model("qwen", ["text"]);
  const embed = model("nomic", ["embedding"]);
  const none = { tag: "all", runsOnMine: false, query: "", installed: [] as InstalledModel[] };

  it("keeps everything with no filter", () => {
    expect(filterCatalog(catalogRecipes(all), none)).toHaveLength(4);
  });
  it("narrows by tag", () => {
    expect(filterCatalog(catalogRecipes(all), { ...none, tag: "documents" }).map((r) => r.id)).toEqual(["ask", "notes"]);
  });
  it("runs on what I have needs every slot fillable; no slots always fits", () => {
    expect(runsOnWhatIHave(docs, [text])).toBe(false);
    expect(runsOnWhatIHave(docs, [text, embed])).toBe(true);
    expect(filterCatalog([docs, brief, sample], { ...none, runsOnMine: true, installed: [text] }).map((r) => r.id)).toEqual(["brief", "inbox"]);
  });
  it("searches the name, the description, and the stage whys, ignoring case", () => {
    expect(filterCatalog(catalogRecipes(all), { ...none, query: "RESEARCH" }).map((r) => r.id)).toEqual(["brief"]);
    expect(filterCatalog(catalogRecipes(all), { ...none, query: "ask it anything" }).map((r) => r.id)).toEqual(["ask"]);
    expect(filterCatalog(catalogRecipes(all), { ...none, query: "questions are cheap" }).map((r) => r.id)).toEqual(["ask"]);
    expect(filterCatalog(catalogRecipes(all), { ...none, query: "zzz" })).toEqual([]);
  });
  it("applies all three together", () => {
    expect(filterCatalog(catalogRecipes(all), { tag: "documents", runsOnMine: true, query: "meeting", installed: [text] }).map((r) => r.id)).toEqual(["notes"]);
  });
});

describe("needsLine", () => {
  it("names required inputs by type, text ones by label", () => {
    expect(needsLine(docs)).toBe("a folder, a question");
    expect(needsLine(brief)).toBe("links");
    expect(needsLine(recipe("e", { inputs: { a: { type: "string", label: "Email address" } } }))).toBe("an email address");
    expect(needsLine(recipe("f", { inputs: { a: { type: "file" }, b: { type: "files" }, c: { type: "url" }, d: { type: "text", label: "Instructions" } } }))).toBe("a file, some files, a link, instructions");
  });
  it("skips optional inputs and says nothing when nothing is needed", () => {
    expect(needsLine(recipe("o", { inputs: { a: { type: "file", required: false } } }))).toBe("nothing");
    expect(needsLine(sample)).toBe("nothing");
  });
});
