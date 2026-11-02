import { describe, expect, it } from "vitest";
import { partitionShelf, shelfOf } from "./shelf";
import type { RecipeInfo } from "./types";

function recipe(id: string, extra: Partial<RecipeInfo> = {}): RecipeInfo {
  return { id, name: id, version: null, description: null, source: "builtin", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [], ...extra };
}

describe("partitionShelf", () => {
  it("splits by the engine's shelf state and keeps order within each", () => {
    const p = partitionShelf([
      recipe("a", { shelf: "catalog" }), recipe("b", { shelf: "shelf" }), recipe("c", { shelf: "archive" }),
      recipe("d", { shelf: "shelf", source: "user" }), recipe("e", { shelf: "catalog" }),
    ]);
    expect(p.shelf.map((r) => r.id)).toEqual(["b", "d"]);
    expect(p.catalog.map((r) => r.id)).toEqual(["a", "e"]);
    expect(p.archive.map((r) => r.id)).toEqual(["c"]);
  });

  it("reads a missing shelf as on the shelf, so an older engine keeps working", () => {
    const p = partitionShelf([recipe("old"), recipe("mine", { source: "user" })]);
    expect(p.shelf.map((r) => r.id)).toEqual(["old", "mine"]);
    expect(p.catalog).toEqual([]);
    expect(p.archive).toEqual([]);
  });

  it("never puts a recipe of yours in the catalog", () => {
    expect(shelfOf(recipe("mine", { source: "user", shelf: "catalog" }))).toBe("shelf");
    expect(shelfOf(recipe("mine", { source: "user", shelf: "archive" }))).toBe("archive");
  });
});
