import type { InputSpec, InstalledModel, RecipeInfo, RecipeSource } from "./types";

/* The catalog: every recipe that ships with Stillroom, read and filtered without the engine. Pure
   functions; the Browse place renders what they return. */

/** The word for a recipe's source on a card: built-in, community, sample, yours. An unknown source
 *  (a newer engine) reads as built-in. */
export function sourceWord(source: RecipeSource | string | undefined): string {
  switch (source) {
    case "user": return "yours";
    case "community": return "community";
    case "sample": return "sample";
    default: return "built-in";
  }
}

/** The recipe's tags, none for an older engine. */
export function tagsOf(r: RecipeInfo): string[] {
  return Array.isArray(r.tags) ? r.tags : [];
}

/** The recipes the catalog shows: everything that is not yours, in the engine's order. */
export function catalogRecipes(recipes: readonly RecipeInfo[]): RecipeInfo[] {
  return recipes.filter((r) => r.source !== "user");
}

/** Tags that say where a recipe lives rather than what it is about; they are rows, not filters. */
const ROW_TAGS = new Set(["built-in", "example", "sample"]);

/** The distinct tags worth filtering by, in order of first appearance. */
export function catalogTags(recipes: readonly RecipeInfo[]): string[] {
  const seen = new Set<string>();
  for (const r of recipes) for (const t of tagsOf(r)) if (!ROW_TAGS.has(t)) seen.add(t);
  return Array.from(seen);
}

/** True when every model slot can be filled by one of the installed models. A recipe with no slots runs anywhere. */
export function runsOnWhatIHave(r: RecipeInfo, installed: readonly InstalledModel[]): boolean {
  return Object.values(r.modelSlots).every((s) => installed.some((m) => s.requires.every((c) => m.capabilities.includes(c))));
}

export interface CatalogFilter {
  /** A tag from `catalogTags`, or "all". */
  tag: string;
  /** Keep only recipes every slot of which an installed model can fill. */
  runsOnMine: boolean;
  /** Matched against the name, the description, and each stage's why; case does not matter. */
  query: string;
  installed: readonly InstalledModel[];
}

function haystack(r: RecipeInfo): string {
  return [r.name, r.description ?? "", ...r.stages.map((s) => s.why ?? "")].join("\n").toLowerCase();
}

/** The catalog narrowed by a tag, a model test, and a search, in the order given. */
export function filterCatalog(recipes: readonly RecipeInfo[], filter: CatalogFilter): RecipeInfo[] {
  const q = filter.query.trim().toLowerCase();
  return recipes.filter((r) => {
    if (filter.tag !== "all" && !tagsOf(r).includes(filter.tag)) return false;
    if (filter.runsOnMine && !runsOnWhatIHave(r, filter.installed)) return false;
    if (q && !haystack(r).includes(q)) return false;
    return true;
  });
}

export interface CatalogRows {
  builtin: RecipeInfo[];
  community: RecipeInfo[];
  sample: RecipeInfo[];
}

/** The catalog in its three rows: built-in, community, samples. Anything else reads as built-in. */
export function catalogRows(recipes: readonly RecipeInfo[]): CatalogRows {
  const rows: CatalogRows = { builtin: [], community: [], sample: [] };
  for (const r of recipes) {
    if (r.source === "community") rows.community.push(r);
    else if (r.source === "sample") rows.sample.push(r);
    else if (r.source !== "user") rows.builtin.push(r);
  }
  return rows;
}

/** "a" or "an" before a word; none before a plural ("instructions"). */
function article(word: string): string {
  if (/[^s]s$/i.test(word)) return word;
  return /^[aeiou]/i.test(word) ? `an ${word}` : `a ${word}`;
}

/** One required input in words: a folder, links, a question. A text input is named by its label. */
export function inputWord(key: string, spec: InputSpec): string {
  switch (spec.type) {
    case "file": return "a file";
    case "files": return "some files";
    case "directory": return "a folder";
    case "url": return "a link";
    case "urls": return "links";
    case "number": return "a number";
    case "boolean": return "a yes or no";
    case "json": return "some data";
    default: {
      const label = (spec.label ?? key).trim();
      return label ? article(label.toLowerCase()) : "some text";
    }
  }
}

/** What a recipe needs to run, as one phrase: "a folder, a question"; "nothing" when it asks for nothing. */
export function needsLine(r: RecipeInfo): string {
  const words = Object.entries(r.inputs).filter(([, i]) => i.required !== false).map(([k, i]) => inputWord(k, i));
  return words.length ? words.join(", ") : "nothing";
}
