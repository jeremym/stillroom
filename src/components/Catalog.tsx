import { useMemo, useState, type ReactNode } from "react";
import { Empty, Segmented, Toggle, Zone } from "@overtone-foundry/app-kit";
import { Glyph } from "../brand";
import { api } from "../api";
import { catalogRecipes, catalogRows, catalogTags, filterCatalog, needsLine, type CatalogFilter } from "../catalog";
import { describePermissions } from "../permissions";
import { shelfOf } from "../shelf";
import { RecipeCard, type CardFacts } from "./Shelf";
import type { RecipeInfo } from "../types";

/* The Catalog place (⌘B): every recipe that ships with Stillroom, in three rows, with a filter row on
   top. A card says what the recipe needs and what it may touch, and offers Add to shelf and Look.
   Nothing runs from here; a recipe is shelved first, so the shelf always tells the truth. */

interface Props {
  /** Every recipe the engine knows; the catalog is the part that is not yours. */
  recipes: readonly RecipeInfo[];
  facts: CardFacts;
  /** Look: open the recipe's page in its read-only form. */
  onLook: (id: string) => void;
  onError: (message: string) => void;
  /** For the plain-browser dev build, where `shelf:changed` never arrives. */
  onChanged?: () => void;
}

export function CatalogPlace({ recipes, facts, onLook, onError, onChanged }: Props) {
  const [tag, setTag] = useState("all");
  const [runsOnMine, setRunsOnMine] = useState(false);
  const [query, setQuery] = useState("");
  const all = useMemo(() => catalogRecipes(recipes), [recipes]);
  const tags = useMemo(() => catalogTags(all), [all]);
  const shown = useMemo(() => {
    const filter: CatalogFilter = { tag, runsOnMine, query, installed: facts.models };
    return filterCatalog(all, filter);
  }, [all, tag, runsOnMine, query, facts.models]);
  const rows = catalogRows(shown);
  const filtering = tag !== "all" || runsOnMine || query.trim() !== "";
  const clear = () => { setTag("all"); setRunsOnMine(false); setQuery(""); };
  const add = (id: string) => api.setShelf(id, "shelf").then(onChanged).catch((e) => onError(String(e)));
  const showSamples = () => api.revealSampleFolders().catch((e) => onError(String(e)));

  const card = (r: RecipeInfo, provenance?: ReactNode) => (
    <CatalogCard key={r.id} recipe={r} facts={facts} provenance={provenance} onLook={onLook} onAdd={add} />
  );

  return (
    <>
      <Zone title="Catalog" sub="every recipe that ships with Stillroom; add one to your shelf to use it" className="accent"
        aside={<span>{all.length} recipe{all.length === 1 ? "" : "s"}{filtering ? `, ${shown.length} shown` : ""}</span>}>
        <div className="catalog-filters">
          <Segmented label="Kind of recipe" value={tag} onChange={setTag}
            options={[{ value: "all", label: "all" }, ...tags.map((t) => ({ value: t, label: t }))]} />
          <label className="catalog-toggle">
            <Toggle on={runsOnMine} onChange={setRunsOnMine} label="Runs on what I have" />
            <span>runs on what I have <span className="muted">(every model it needs is installed)</span></span>
          </label>
          <input type="search" className="catalog-search" value={query} onChange={(e) => setQuery(e.target.value)}
            placeholder="Search names, descriptions, and why each stage is there" aria-label="Search the catalog" />
        </div>
      </Zone>

      {shown.length === 0 && (
        <Zone>
          <Empty word={all.length === 0 ? "Nothing in the catalog" : "Nothing matches"}>
            {all.length === 0
              ? <p>This build shipped without recipes. Make one with New recipe.</p>
              : <><p>No recipe fits these filters.</p><button onClick={clear}>Clear the filters</button></>}
          </Empty>
        </Zone>
      )}

      {rows.builtin.length > 0 && (
        <Zone title="Built-in" sub="written here, kept up to date with the app" aside={<span>{rows.builtin.length}</span>}>
          <div className="cards recipe-grid">{rows.builtin.map((r) => card(r))}</div>
        </Zone>
      )}

      {rows.community.length > 0 && (
        <Zone title="Community" sub="contributed through the repository, reviewed before they shipped" aside={<span>{rows.community.length}</span>}>
          <div className="cards recipe-grid">
            {rows.community.map((r) => card(r, <>{r.author?.name ? <>by <b>{r.author.name}</b> · </> : null}reviewed before it shipped</>))}
          </div>
        </Zone>
      )}

      {rows.sample.length > 0 && (
        <Zone title="Samples" sub="need the sample folders; copy them first"
          aside={<button className="quiet" onClick={showSamples}><Glyph name="folder" /> Show the sample folders</button>}>
          <p className="muted">A sample recipe reads folders that ship beside the app. Copy those folders somewhere of your own and point the recipe at the copy; the originals are not meant to be worked on.</p>
          <div className="cards recipe-grid">{rows.sample.map((r) => card(r, <>needs the sample folders; copy them first</>))}</div>
        </Zone>
      )}
    </>
  );
}

interface CardProps {
  recipe: RecipeInfo;
  facts: CardFacts;
  /** One more line under the card: who wrote it, what it needs beside its inputs. */
  provenance?: ReactNode;
  onLook: (id: string) => void;
  onAdd: (id: string) => void;
}

/** A shelf card with two more lines (needs, may touch) and the catalog's two buttons. */
export function CatalogCard({ recipe: r, facts, provenance, onLook, onAdd }: CardProps) {
  const shelved = shelfOf(r) === "shelf";
  return (
    <RecipeCard recipe={r} facts={facts} onOpen={onLook}
      lines={
        <>
          <span><span className="label">needs</span> {needsLine(r)}</span>
          <span><span className="label">may touch</span> {describePermissions(r.permissions, r.inputs)}</span>
          {provenance && <span className="muted">{provenance}</span>}
        </>
      }
      foot={
        <div className="card-foot">
          {shelved
            ? <span className="tag" title="Already on your shelf">on the shelf</span>
            : <button className="quiet" onClick={() => onAdd(r.id)}><Glyph name="plus" /> Add to shelf</button>}
          <button className="quiet" onClick={() => onLook(r.id)}><Glyph name="recipe" /> Look</button>
        </div>
      } />
  );
}
