import { useMemo, useState } from "react";
import { Glyph, RECIPE_GLYPHS, Zone, type GlyphName } from "../kit";
import { slugify, uniqueId } from "../recipes/new";
import type { RecipeInfo } from "../types";

/* New recipe: a starting point, a name, an icon. Then the map opens in edit mode. */
export interface NewRecipeRequest { mode: "blank" | "copy"; sourceId?: string; id: string; name: string; icon: GlyphName }

interface Props {
  recipes: RecipeInfo[];
  busy: boolean;
  error: string | null;
  onCreate: (req: NewRecipeRequest) => void;
  onCancel: () => void;
}

export function NewRecipe({ recipes, busy, error, onCreate, onCancel }: Props) {
  const [mode, setMode] = useState<"blank" | "copy">("blank");
  const [sourceId, setSourceId] = useState(recipes[0]?.id ?? "");
  const [name, setName] = useState("");
  const [icon, setIcon] = useState<GlyphName>("recipe");
  const taken = useMemo(() => recipes.map((r) => r.id), [recipes]);
  const id = uniqueId(slugify(name), taken);
  const ready = name.trim().length > 0 && (mode === "blank" || Boolean(sourceId));

  return (
    <Zone title="New recipe" sub="a reusable process you can run, watch, and share" className="accent">
      <h3>Start from</h3>
      <div className="choices">
        <button className="choice" aria-pressed={mode === "blank"} onClick={() => setMode("blank")}>
          <span><Glyph name="file" /> <strong>Blank</strong></span>
          <span className="dim">Reads one file and hands it back. Add steps on the map.</span>
        </button>
        <button className="choice" aria-pressed={mode === "copy"} onClick={() => setMode("copy")}>
          <span><Glyph name="copy" /> <strong>A copy of a recipe</strong></span>
          <span className="dim">Everything from an existing recipe, under your name.</span>
        </button>
        <button className="choice" disabled title="Coming with the planner">
          <span><Glyph name="sparkles" /> <strong>From a goal</strong></span>
          <span className="dim">Describe what you want; a draft appears. Coming with the planner.</span>
        </button>
      </div>
      {mode === "copy" && (
        <div className="field">
          <label><strong>Copy of</strong></label>
          <select value={sourceId} onChange={(e) => setSourceId(e.target.value)}>
            {recipes.map((r) => <option key={r.id} value={r.id}>{r.name}{r.source === "builtin" ? " (built-in)" : ""}</option>)}
          </select>
        </div>
      )}

      <div className="field">
        <label><strong>Name</strong> <span className="muted">· id will be <span className="mono">{id || "…"}</span></span></label>
        <input type="text" value={name} placeholder="Weekly report" autoFocus onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && ready && !busy) onCreate({ mode, sourceId, id, name: name.trim(), icon }); }} />
      </div>

      <div className="field">
        <label><strong>Icon</strong> <span className="muted">· shown on the card and in the breadcrumb</span></label>
        <div className="icon-grid" role="radiogroup" aria-label="Icon">
          {RECIPE_GLYPHS.map((g) => (
            <button key={g} type="button" role="radio" aria-checked={icon === g} aria-pressed={icon === g} title={g} onClick={() => setIcon(g)}><Glyph name={g} /></button>
          ))}
        </div>
      </div>

      {error && <pre className="error">{error}</pre>}
      <div className="row">
        <button className="primary" style={{ marginTop: 0 }} disabled={!ready || busy} onClick={() => onCreate({ mode, sourceId, id, name: name.trim(), icon })}>
          {busy ? "Creating…" : "Create and open its card"}
        </button>
        <button disabled={busy} onClick={onCancel}>Cancel</button>
      </div>
    </Zone>
  );
}
