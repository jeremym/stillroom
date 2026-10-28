import { useState } from "react";
import type { Deck } from "./SlidesView";
import { isDeck } from "./SlidesView";
import type { Script } from "./AudioView";
import { isScript } from "./AudioView";
import { asDiagram } from "./DiagramView";

/* Editors for the artifacts a user is most likely to touch by hand. Anything else falls back
   to raw JSON. Saving hands the new value back as that step's output (change propagation). */

interface Concept { id: string; name: string; definition: string; prerequisites: string[]; related: string[]; evidence: string[] }
interface Claim { id: string; text: string; concepts: string[]; evidence: string }
interface Spine { title: string; summary: string; concepts: Concept[]; claims: Claim[] }
interface Card { front: string; back: string; concept: string }

function isSpine(v: unknown): v is Spine {
  return typeof v === "object" && v !== null && Array.isArray((v as { concepts?: unknown }).concepts) && Array.isArray((v as { claims?: unknown }).claims);
}
function isCards(v: unknown): v is { cards: Card[] } {
  return typeof v === "object" && v !== null && Array.isArray((v as { cards?: unknown }).cards);
}

interface Props {
  kind: string;
  value: unknown;
  /** Human names of the steps that will be regenerated when this is saved. */
  dependents: string[];
  busy: boolean;
  onSave: (output: unknown) => void;
  onCancel: () => void;
}

export function ArtifactEditor({ kind, value, dependents, busy, onSave, onCancel }: Props) {
  const [draft, setDraft] = useState<unknown>(() => structuredClone(value));
  const [jsonText, setJsonText] = useState(() => JSON.stringify(value, null, 2));
  const [jsonError, setJsonError] = useState<string | null>(null);

  const structured =
    typeof draft === "string" ? "text"
    : isScript(draft) ? "script"
    : isSpine(draft) ? "spine"
    : isCards(draft) ? "cards"
    : isDeck(draft) ? "deck"
    : kind === "diagram" && asDiagram(draft) ? "diagram"
    : "json";

  function save() {
    if (structured === "json") {
      try {
        onSave(JSON.parse(jsonText));
      } catch (e) {
        setJsonError(String(e));
      }
      return;
    }
    onSave(draft);
  }

  const summary = dependents.length === 0
    ? "Nothing depends on this, so only this result changes."
    : `Saving regenerates ${dependents.length} step${dependents.length === 1 ? "" : "s"}: ${dependents.join(", ")}. Everything else is reused.`;

  return (
    <div className="editor">
      {structured === "text" && (
        <textarea rows={18} value={draft as string} onChange={(e) => setDraft(e.target.value)} />
      )}
      {structured === "script" && <ScriptEditor script={draft as Script} onChange={setDraft} />}
      {structured === "spine" && <SpineEditor spine={draft as Spine} onChange={setDraft} />}
      {structured === "cards" && <CardsEditor cards={(draft as { cards: Card[] }).cards} onChange={(cards) => setDraft({ cards })} />}
      {structured === "deck" && <DeckEditor deck={draft as Deck} onChange={setDraft} />}
      {structured === "diagram" && (
        <DiagramEditor diagram={draft as { title?: string; source: string }} onChange={setDraft} />
      )}
      {structured === "json" && (
        <>
          <textarea className="mono" rows={18} value={jsonText} onChange={(e) => { setJsonText(e.target.value); setJsonError(null); }} />
          {jsonError && <pre className="error">{jsonError}</pre>}
        </>
      )}
      <p className="muted">{summary}</p>
      <div className="row">
        <button className="primary" style={{ marginTop: 0 }} disabled={busy} onClick={save}>
          {busy ? "Regenerating…" : dependents.length ? "Save and regenerate" : "Save"}
        </button>
        <button disabled={busy} onClick={onCancel}>Cancel</button>
      </div>
    </div>
  );
}

function ScriptEditor({ script, onChange }: { script: Script; onChange: (s: Script) => void }) {
  const update = (i: number, patch: Partial<Script["chapters"][number]>) =>
    onChange({ ...script, chapters: script.chapters.map((c, k) => (k === i ? { ...c, ...patch } : c)) });
  return (
    <div>
      <label className="field"><strong>Title</strong>
        <input type="text" value={script.title} onChange={(e) => onChange({ ...script, title: e.target.value })} />
      </label>
      {script.chapters.map((c, i) => (
        <div className="field edit-block" key={i}>
          <div className="row space">
            <input type="text" value={c.title} onChange={(e) => update(i, { title: e.target.value })} />
            <span className="muted mono">{c.concept}</span>
            <button className="link" onClick={() => onChange({ ...script, chapters: script.chapters.filter((_, k) => k !== i) })}>remove chapter</button>
          </div>
          <textarea rows={5} value={c.text} onChange={(e) => update(i, { text: e.target.value })} />
        </div>
      ))}
    </div>
  );
}

function SpineEditor({ spine, onChange }: { spine: Spine; onChange: (s: Spine) => void }) {
  const updateConcept = (i: number, patch: Partial<Concept>) =>
    onChange({ ...spine, concepts: spine.concepts.map((c, k) => (k === i ? { ...c, ...patch } : c)) });
  const removeConcept = (i: number) => {
    const id = spine.concepts[i].id;
    onChange({
      ...spine,
      concepts: spine.concepts
        .filter((_, k) => k !== i)
        .map((c) => ({ ...c, prerequisites: c.prerequisites.filter((p) => p !== id), related: c.related.filter((r) => r !== id) })),
      claims: spine.claims.map((cl) => ({ ...cl, concepts: cl.concepts.filter((c) => c !== id) })),
    });
  };
  const updateClaim = (i: number, text: string) =>
    onChange({ ...spine, claims: spine.claims.map((c, k) => (k === i ? { ...c, text } : c)) });
  return (
    <div>
      <label className="field"><strong>Title</strong>
        <input type="text" value={spine.title} onChange={(e) => onChange({ ...spine, title: e.target.value })} />
      </label>
      <label className="field"><strong>Summary</strong>
        <textarea rows={3} value={spine.summary} onChange={(e) => onChange({ ...spine, summary: e.target.value })} />
      </label>
      <h4>Concepts <span className="muted">({spine.concepts.length})</span></h4>
      {spine.concepts.map((c, i) => (
        <div className="field edit-block" key={c.id}>
          <div className="row space">
            <input type="text" value={c.name} onChange={(e) => updateConcept(i, { name: e.target.value })} />
            <span className="muted mono">{c.id}</span>
            <button className="link" onClick={() => removeConcept(i)}>remove concept</button>
          </div>
          <textarea rows={3} value={c.definition} onChange={(e) => updateConcept(i, { definition: e.target.value })} />
          {(c.prerequisites.length > 0 || c.related.length > 0) && (
            <div className="muted">
              {c.prerequisites.length > 0 && <>needs: {c.prerequisites.join(", ")} </>}
              {c.related.length > 0 && <>· related: {c.related.join(", ")}</>}
            </div>
          )}
        </div>
      ))}
      {spine.claims.length > 0 && (
        <>
          <h4>Claims <span className="muted">({spine.claims.length})</span></h4>
          {spine.claims.map((cl, i) => (
            <div className="field edit-block" key={cl.id}>
              <div className="row space">
                <span className="muted mono">{cl.id} · {cl.concepts.join(", ")}</span>
                <button className="link" onClick={() => onChange({ ...spine, claims: spine.claims.filter((_, k) => k !== i) })}>remove claim</button>
              </div>
              <textarea rows={2} value={cl.text} onChange={(e) => updateClaim(i, e.target.value)} />
            </div>
          ))}
        </>
      )}
    </div>
  );
}

function CardsEditor({ cards, onChange }: { cards: Card[]; onChange: (c: Card[]) => void }) {
  const update = (i: number, patch: Partial<Card>) => onChange(cards.map((c, k) => (k === i ? { ...c, ...patch } : c)));
  return (
    <div>
      {cards.map((c, i) => (
        <div className="field edit-block" key={i}>
          <div className="row space">
            <span className="muted mono">card {i + 1} · {c.concept}</span>
            <button className="link" onClick={() => onChange(cards.filter((_, k) => k !== i))}>remove card</button>
          </div>
          <textarea rows={2} value={c.front} placeholder="question" onChange={(e) => update(i, { front: e.target.value })} />
          <textarea rows={2} value={c.back} placeholder="answer" onChange={(e) => update(i, { back: e.target.value })} />
        </div>
      ))}
      <button onClick={() => onChange([...cards, { front: "", back: "", concept: cards[0]?.concept ?? "" }])}>Add card</button>
    </div>
  );
}

function DeckEditor({ deck, onChange }: { deck: Deck; onChange: (d: Deck) => void }) {
  const update = (i: number, patch: Partial<Deck["slides"][number]>) =>
    onChange({ ...deck, slides: deck.slides.map((s, k) => (k === i ? { ...s, ...patch } : s)) });
  return (
    <div>
      <label className="field"><strong>Title</strong>
        <input type="text" value={deck.title} onChange={(e) => onChange({ ...deck, title: e.target.value })} />
      </label>
      {deck.slides.map((s, i) => (
        <div className="field edit-block" key={i}>
          <div className="row space">
            <input type="text" value={s.title} onChange={(e) => update(i, { title: e.target.value })} />
            <span className="muted mono">slide {i + 1} · {s.concept}</span>
            <button className="link" onClick={() => onChange({ ...deck, slides: deck.slides.filter((_, k) => k !== i) })}>remove slide</button>
          </div>
          <textarea rows={4} value={s.bullets.join("\n")} placeholder="one bullet per line"
            onChange={(e) => update(i, { bullets: e.target.value.split("\n").filter((b) => b.trim()) })} />
          <textarea rows={3} value={s.notes} placeholder="speaker notes" onChange={(e) => update(i, { notes: e.target.value })} />
        </div>
      ))}
    </div>
  );
}

function DiagramEditor({ diagram, onChange }: { diagram: { title?: string; source: string }; onChange: (d: unknown) => void }) {
  return (
    <div>
      {diagram.title !== undefined && (
        <label className="field"><strong>Title</strong>
          <input type="text" value={diagram.title} onChange={(e) => onChange({ ...diagram, title: e.target.value })} />
        </label>
      )}
      <label className="field"><strong>Mermaid source</strong>
        <textarea className="mono" rows={16} value={diagram.source} onChange={(e) => onChange({ ...diagram, source: e.target.value })} />
      </label>
    </div>
  );
}
