import { useEffect, useRef, useState, type ReactNode } from "react";
import { Card, Empty, Zone, type TagTone } from "@overtone-foundry/app-kit";
import { Glyph, inputGlyph, isGlyphName } from "../brand";
import { api } from "../api";
import type { InstalledModel, RecipeInfo, RunSummary } from "../types";

/* The Library's recipe cards and the two zones under them. A card is the kit's Card; what differs
   is the row under it: a small menu on a shelved card, Add to shelf on a catalog card, Restore on an
   archived one. Changing a recipe's shelf state goes to the engine, which answers with
   `shelf:changed`; `onChanged` is for the plain-browser dev build where that event never comes. */

export interface CardFacts {
  models: InstalledModel[];
  history: RunSummary[];
}

/** The tag row every recipe card shares: steps, required inputs, whose it is, a model warning, the last run. */
export function cardTags(r: RecipeInfo, facts: CardFacts): { label: ReactNode; tone?: TagTone }[] {
  const fillable = Object.values(r.modelSlots).every((s) => facts.models.some((m) => s.requires.every((c) => m.capabilities.includes(c as never))));
  const lastRun = facts.history.find((h) => h.recipeId === r.id);
  const tags: { label: ReactNode; tone?: TagTone }[] = [{ label: `${r.steps.length} steps` }];
  for (const i of Object.values(r.inputs).filter((i) => i.required !== false).slice(0, 3)) {
    tags.push({ label: <><Glyph name={inputGlyph(i.type)} /> {i.label ?? i.type}</> });
  }
  tags.push({ label: r.source === "builtin" ? "built-in" : "yours" });
  if (!fillable) tags.push({ label: "needs a model", tone: "amber" });
  if (lastRun) tags.push({ label: `last run ${new Date(lastRun.startedAt).toLocaleDateString()}` });
  return tags;
}

interface CardProps {
  recipe: RecipeInfo;
  facts: CardFacts;
  onOpen: (id: string) => void;
  /** The row under the card: a menu, Add to shelf, Restore. */
  foot?: ReactNode;
}

/** One recipe as a card; the card itself opens the recipe, the foot holds whatever acts on it. */
export function RecipeCard({ recipe: r, facts, onOpen, foot }: CardProps) {
  return (
    <div className="card-slot">
      <Card title={<><Glyph name={isGlyphName(r.icon) ? r.icon : "recipe"} /> {r.name}</>} tags={cardTags(r, facts)} onClick={() => onOpen(r.id)}>
        {r.description || undefined}
      </Card>
      {foot}
    </div>
  );
}

export interface MenuActions {
  onSetup: (id: string) => void;
  onDoor: (id: string) => void;
  onDuplicate: (r: RecipeInfo) => void;
  onDelete: (r: RecipeInfo) => void;
  onError: (message: string) => void;
  onChanged?: () => void;
}

/** The ⋯ menu on a shelved card: Set up a run · Recipe card · Duplicate · Archive · Delete · Reveal in Finder. */
export function CardMenu({ recipe: r, onSetup, onDoor, onDuplicate, onDelete, onError, onChanged }: { recipe: RecipeInfo } & MenuActions) {
  const ref = useRef<HTMLDetailsElement>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const yours = r.source === "user";
  const close = () => { if (ref.current) ref.current.open = false; setConfirmDelete(false); };
  const act = (f: () => void) => () => { close(); f(); };
  // A click anywhere else folds the menu, as a menu is expected to.
  useEffect(() => {
    const h = (e: MouseEvent) => { if (ref.current?.open && !ref.current.contains(e.target as Node)) close(); };
    document.addEventListener("mousedown", h);
    return () => document.removeEventListener("mousedown", h);
  }, []);
  return (
    <details className="card-menu" ref={ref} onToggle={() => { if (!ref.current?.open) setConfirmDelete(false); }}>
      <summary aria-label={`More for ${r.name}`} title="More">⋯</summary>
      <div className="menu" role="menu">
        <button role="menuitem" onClick={act(() => onSetup(r.id))}><Glyph name="play" /> Set up a run</button>
        <button role="menuitem" onClick={act(() => onDoor(r.id))}><Glyph name="door" /> Recipe card</button>
        <button role="menuitem" onClick={act(() => onDuplicate(r))}><Glyph name="copy" /> Duplicate</button>
        <button role="menuitem" onClick={act(() => api.setShelf(r.id, "archive").then(onChanged).catch((e) => onError(String(e))))}><Glyph name="inbox" /> Archive</button>
        {yours && (confirmDelete
          ? <div className="menu-confirm">
              <span className="muted">Also deletes what it kept (its run keeps).</span>
              <span className="row">
                <button className="danger" onClick={act(() => onDelete(r))}>Really delete</button>
                <button onClick={() => setConfirmDelete(false)}>Keep it</button>
              </span>
            </div>
          : <button role="menuitem" className="danger-item" onClick={() => setConfirmDelete(true)}><Glyph name="delete" /> Delete…</button>)}
        {yours && <button role="menuitem" onClick={act(() => api.revealRecipesFolder().catch((e) => onError(String(e))))}><Glyph name="folder" /> Reveal in Finder</button>}
      </div>
    </details>
  );
}

/** The fold on a zone: "Show (n)" or "Hide". */
function Disclosure({ open, count, onToggle }: { open: boolean; count: number; onToggle: (open: boolean) => void }) {
  return (
    <button className="quiet" aria-expanded={open} onClick={() => onToggle(!open)}>
      <Glyph name="down" className={open ? "turned" : ""} /> {open ? "Hide" : `Show (${count})`}
    </button>
  );
}

interface ZoneProps {
  recipes: RecipeInfo[];
  facts: CardFacts;
  onOpen: (id: string) => void;
  onError: (message: string) => void;
  onChanged?: () => void;
}

/** Every recipe that ships with Stillroom and is not on the shelf. Folded until asked for. */
export function CatalogZone({ recipes, facts, onOpen, onError, onChanged, open, onToggle }: ZoneProps & { open: boolean; onToggle: (open: boolean) => void }) {
  if (recipes.length === 0) return null;
  const add = (id: string) => api.setShelf(id, "shelf").then(onChanged).catch((e) => onError(String(e)));
  return (
    <Zone id="catalog" title="Catalog" sub="every recipe that ships with Stillroom; add one to your shelf to use it"
      aside={<Disclosure open={open} count={recipes.length} onToggle={onToggle} />}>
      {open && (
        <div className="cards recipe-grid">
          {recipes.map((r) => (
            <RecipeCard key={r.id} recipe={r} facts={facts} onOpen={onOpen}
              foot={<div className="card-foot"><button className="quiet" onClick={() => add(r.id)}><Glyph name="plus" /> Add to shelf</button></div>} />
          ))}
        </div>
      )}
    </Zone>
  );
}

/** Recipes taken off the shelf. Nothing here is deleted; Restore puts one back. */
export function ArchiveZone({ recipes, facts, onOpen, onError, onChanged }: ZoneProps) {
  const [open, setOpen] = useState(false);
  if (recipes.length === 0) return null;
  const restore = (id: string) => api.setShelf(id, "shelf").then(onChanged).catch((e) => onError(String(e)));
  return (
    <Zone id="archive" title="Archive" sub="taken off the shelf; runs, watches, and keeps are kept"
      aside={<Disclosure open={open} count={recipes.length} onToggle={setOpen} />}>
      {open && (
        <div className="cards recipe-grid">
          {recipes.map((r) => (
            <RecipeCard key={r.id} recipe={r} facts={facts} onOpen={onOpen}
              foot={<div className="card-foot"><button className="quiet" onClick={() => restore(r.id)}><Glyph name="back" /> Restore</button></div>} />
          ))}
        </div>
      )}
    </Zone>
  );
}

/** What the Recipes zone shows with nothing on the shelf. */
export function EmptyShelf({ onBrowse, onNew }: { onBrowse: () => void; onNew: () => void }) {
  return (
    <Empty word="Nothing on the shelf">
      <p>Browse the catalog, or make a recipe.</p>
      <div className="row center">
        <button className="primary" style={{ marginTop: 0 }} onClick={onBrowse}><Glyph name="search" /> Browse the catalog</button>
        <button onClick={onNew}><Glyph name="plus" /> New recipe</button>
      </div>
    </Empty>
  );
}
