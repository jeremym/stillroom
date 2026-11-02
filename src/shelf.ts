import type { RecipeInfo, ShelfState } from "./types";

/* The shelf: which recipes the Library shows in front, which are folded under Archive, and which
   wait in the catalog. The engine says; an older engine that says nothing means "on the shelf". */

export function shelfOf(r: RecipeInfo): ShelfState {
  // A recipe of yours cannot be in the catalog; an engine that never heard of the shelf puts everything on it.
  if (r.shelf === "catalog" && r.source === "user") return "shelf";
  return r.shelf ?? "shelf";
}

export interface ShelfPartition {
  shelf: RecipeInfo[];
  catalog: RecipeInfo[];
  archive: RecipeInfo[];
}

/** Split the recipe list three ways, keeping the engine's order within each. */
export function partitionShelf(recipes: readonly RecipeInfo[]): ShelfPartition {
  const out: ShelfPartition = { shelf: [], catalog: [], archive: [] };
  for (const r of recipes) out[shelfOf(r)].push(r);
  return out;
}
