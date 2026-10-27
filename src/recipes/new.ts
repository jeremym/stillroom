import type { Draft, RecipeDoc } from "../types";

/* Making a recipe in the app: a blank one that already runs, or a copy of an existing one under a
   new name. Pure functions; the Rust checker still has the last word when the draft is saved. */

/** "My Weekly Report!" → "my-weekly-report". */
export function slugify(name: string): string {
  return name
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}

/** An id that is not already taken: "report", then "report-2", "report-3". */
export function uniqueId(base: string, taken: Iterable<string>): string {
  const set = new Set(taken);
  const root = base || "recipe";
  if (!set.has(root)) return root;
  for (let n = 2; ; n++) {
    const id = `${root}-${n}`;
    if (!set.has(id)) return id;
  }
}

/** The smallest recipe that runs: read a file, hand it back. Everything else is edited on the map. */
export function blankRecipe(id: string, name: string, icon?: string): Draft {
  const yaml: RecipeDoc = {
    schemaVersion: "0",
    id,
    name,
    version: "0.1.0",
    ...(icon ? { icon } : {}),
    description: "A new recipe. Open its recipe card to add steps.",
    inputs: {
      source: { type: "file", label: "Source file", description: "The file this recipe starts from." },
    },
    permissions: { filesystem: { read: ["input"] } } as RecipeDoc["permissions"],
    steps: [
      { id: "read", kind: "tool", name: "Read the file", tool: "file.read", inputs: { path: "${inputs.source}" } },
    ],
    outputs: {
      text: { from: "${steps.read.output.text}", type: "text", label: "The file's text" },
    },
  };
  return { yaml, files: {} };
}

/** A copy of a draft under a new id, name, and icon. A copy is yours whatever it was copied from. */
export function copyRecipe(draft: Draft, id: string, name: string, icon?: string | null, fromBuiltin = false): Draft {
  const yaml: RecipeDoc = structuredClone(draft.yaml);
  yaml.id = id;
  yaml.name = name;
  if (icon) yaml.icon = icon; else if (icon === null) delete yaml.icon;
  if (Array.isArray(yaml.tags)) yaml.tags = (yaml.tags as string[]).filter((t) => t !== "built-in");
  if (fromBuiltin) delete yaml.author;
  return { yaml, files: { ...draft.files } };
}
