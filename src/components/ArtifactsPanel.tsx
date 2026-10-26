import { useState } from "react";
import Markdown from "react-markdown";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { DiagramView, asDiagram } from "./DiagramView";
import type { OutputSpec, Run } from "../types";

interface Concept { id: string; name: string; definition: string; prerequisites: string[]; related: string[]; evidence: string[] }
interface Claim { id: string; text: string; concepts: string[]; evidence: string }
interface Card { front: string; back: string; concept: string }

function isSpine(v: unknown): v is { title: string; summary: string; concepts: Concept[]; claims: Claim[] } {
  return typeof v === "object" && v !== null && Array.isArray((v as { concepts?: unknown }).concepts);
}
function isCards(v: unknown): v is { cards: Card[] } {
  return typeof v === "object" && v !== null && Array.isArray((v as { cards?: unknown }).cards);
}

function Spine({ spine }: { spine: { title: string; summary: string; concepts: Concept[]; claims: Claim[] } }) {
  const byId = new Map(spine.concepts.map((c) => [c.id, c.name]));
  const name = (id: string) => byId.get(id) ?? id;
  return (
    <div className="spine">
      <p><strong>{spine.title}</strong> · {spine.summary}</p>
      <ul className="concepts">
        {spine.concepts.map((c) => (
          <li key={c.id}>
            <strong>{c.name}</strong> — {c.definition}
            {c.prerequisites.length > 0 && <div className="muted">needs: {c.prerequisites.map(name).join(", ")}</div>}
            {c.related.length > 0 && <div className="muted">related: {c.related.map(name).join(", ")}</div>}
            {c.evidence[0] && <div className="quote">“{c.evidence[0]}”</div>}
          </li>
        ))}
      </ul>
      {spine.claims.length > 0 && (
        <>
          <h4>Claims <span className="muted">({spine.claims.length})</span></h4>
          <ul>
            {spine.claims.map((cl) => (
              <li key={cl.id}>{cl.text} <span className="muted">· {cl.concepts.map(name).join(", ")}</span></li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

function Cards({ cards }: { cards: Card[] }) {
  const [flipped, setFlipped] = useState<Record<number, boolean>>({});
  return (
    <div className="cards">
      {cards.map((c, i) => (
        <button key={i} className={`card ${flipped[i] ? "back" : ""}`} onClick={() => setFlipped((f) => ({ ...f, [i]: !f[i] }))}>
          <div className="card-face">{flipped[i] ? c.back : c.front}</div>
          <div className="muted mono">{flipped[i] ? "answer" : "question"} · {c.concept}</div>
        </button>
      ))}
    </div>
  );
}

function Artifact({ name, spec, value }: { name: string; spec: OutputSpec | undefined; value: unknown }) {
  const label = spec?.label ?? name;
  const kind = spec?.type ?? (typeof value === "string" ? "text" : "json");
  const diagram = kind === "diagram" ? asDiagram(value) : null;
  const [svg, setSvg] = useState<string | null>(null);
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  const ext = kind === "markdown" ? "md" : kind === "json" ? "json" : kind === "diagram" ? (svg ? "svg" : "mmd") : "txt";

  async function exportFile() {
    const path = await save({ defaultPath: `${name}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
    if (!path) return;
    const body = diagram ? (svg ?? diagram.source) : text;
    await api.writeTextFile(path, body);
  }

  return (
    <section className="panel">
      <div className="row space">
        <h2 style={{ margin: 0 }}>{label} <span className="muted">(artifact · {kind})</span></h2>
        <button onClick={exportFile}>Save…</button>
      </div>
      {spec?.description && <p className="muted">{spec.description}</p>}
      {diagram ? (
        <>
          {diagram.title && <p><strong>{diagram.title}</strong></p>}
          <DiagramView diagram={diagram} onSvg={setSvg} />
        </>
      ) : kind === "markdown" && typeof value === "string" ? (
        <div className="markdown"><Markdown>{value}</Markdown></div>
      ) : isSpine(value) ? (
        <Spine spine={value} />
      ) : isCards(value) ? (
        <Cards cards={value.cards} />
      ) : typeof value === "string" ? (
        <pre className="artifact">{value}</pre>
      ) : (
        <pre>{text}</pre>
      )}
    </section>
  );
}

export function ArtifactsPanel({ run, outputs }: { run: Run; outputs: Record<string, OutputSpec> }) {
  const entries = Object.entries(run.outputs ?? {});
  if (entries.length === 0) return null;
  return (
    <>
      {entries.map(([name, value]) => (
        <Artifact key={name} name={name} spec={outputs[name]} value={value} />
      ))}
    </>
  );
}
