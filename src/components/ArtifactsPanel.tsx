import { useEffect, useState } from "react";
import Markdown from "react-markdown";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { DiagramView, asDiagram } from "./DiagramView";
import { SlidesView, isDeck, deckToHtml, deckToPptx } from "./SlidesView";
import { AudioView, ScriptView, isAudio, isScript } from "./AudioView";
import { ArtifactEditor } from "./ArtifactEditor";
import type { OutputSpec, Run, StepInfo } from "../types";

/** The step an output comes straight from, if `from` is `${steps.<id>.output}` with no deeper path. */
function sourceStep(spec: OutputSpec | undefined): string | null {
  const m = spec?.from.match(/^\$\{steps\.([A-Za-z0-9_-]+)\.output\}$/);
  return m ? m[1] : null;
}

/** Every step downstream of `stepId`, transitively, in recipe order. */
function dependentsOf(steps: StepInfo[], stepId: string): StepInfo[] {
  const affected = new Set([stepId]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const s of steps) {
      if (!affected.has(s.id) && s.dependsOn.some((d) => affected.has(d))) {
        affected.add(s.id);
        grew = true;
      }
    }
  }
  return steps.filter((s) => s.id !== stepId && affected.has(s.id));
}

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

interface ExportOption { label: string; ext: string; build: () => Promise<string | Uint8Array | { copyFrom: string }>; }

interface ArtifactProps {
  name: string;
  spec: OutputSpec | undefined;
  value: unknown;
  steps: StepInfo[];
  busy: boolean;
  onEdit?: (stepId: string, output: unknown) => void;
}

function Artifact({ name, spec, value, steps, busy, onEdit }: ArtifactProps) {
  const label = spec?.label ?? name;
  const stepId = sourceStep(spec);
  const editable = Boolean(onEdit && stepId && kindIsEditable(spec?.type, value));
  const [editing, setEditing] = useState(false);
  const dependents = stepId ? dependentsOf(steps, stepId).map((s) => s.name ?? s.id) : [];
  const kind = spec?.type ?? (typeof value === "string" ? "text" : "json");
  const diagram = kind === "diagram" ? asDiagram(value) : null;
  const deck = kind === "slides" && isDeck(value) ? value : null;
  const audio = kind === "audio" && isAudio(value) ? value : null;
  const [svg, setSvg] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);

  const exports: ExportOption[] = deck
    ? [
        { label: "Save HTML…", ext: "html", build: async () => deckToHtml(deck) },
        { label: "Save PowerPoint…", ext: "pptx", build: () => deckToPptx(deck) },
        { label: "Save JSON…", ext: "json", build: async () => text },
      ]
    : audio
      ? [{ label: "Save audio…", ext: audio.format ?? "m4a", build: async () => ({ copyFrom: audio.path }) }]
    : diagram
      ? [
          { label: svg ? "Save SVG…" : "Save Mermaid…", ext: svg ? "svg" : "mmd", build: async () => svg ?? diagram.source },
        ]
      : [{ label: "Save…", ext: kind === "markdown" ? "md" : kind === "json" ? "json" : "txt", build: async () => text }];

  async function exportWith(opt: ExportOption) {
    const path = await save({ defaultPath: `${name}.${opt.ext}`, filters: [{ name: opt.ext.toUpperCase(), extensions: [opt.ext] }] });
    if (!path) return;
    setExporting(true);
    try {
      const body = await opt.build();
      if (typeof body === "string") await api.writeTextFile(path, body);
      else if (body instanceof Uint8Array) await api.writeBinaryFile(path, body);
      else await api.copyFile(body.copyFrom, path);
    } finally {
      setExporting(false);
    }
  }

  return (
    <section className="panel">
      <div className="row space">
        <h2 style={{ margin: 0 }}>{label} <span className="muted">(artifact · {kind})</span></h2>
        <span className="row">
          {editable && !editing && <button disabled={busy} onClick={() => setEditing(true)}>Edit…</button>}
          {exports.map((opt) => (
            <button key={opt.ext} disabled={busy || exporting} onClick={() => exportWith(opt)}>{opt.label}</button>
          ))}
        </span>
      </div>
      {spec?.description && <p className="muted">{spec.description}</p>}
      {editing && stepId ? (
        <ArtifactEditor
          kind={kind}
          value={value}
          dependents={dependents}
          busy={busy}
          onCancel={() => setEditing(false)}
          onSave={(output) => { setEditing(false); onEdit?.(stepId, output); }}
        />
      ) : deck ? (
        <SlidesView deck={deck} />
      ) : audio ? (
        <AudioView audio={audio} />
      ) : isScript(value) ? (
        <ScriptView script={value} />
      ) : diagram ? (
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

/** Audio is produced by a tool from a file; edit the script instead. */
function kindIsEditable(kind: string | undefined, value: unknown) {
  if (kind === "audio" || kind === "image" || kind === "video" || kind === "file" || kind === "files") return false;
  return typeof value === "string" || (typeof value === "object" && value !== null);
}

interface PanelProps {
  run: Run;
  outputs: Record<string, OutputSpec>;
  steps: StepInfo[];
  busy: boolean;
  onEdit?: (stepId: string, output: unknown) => void;
}

export function ArtifactsPanel({ run, outputs, steps, busy, onEdit }: PanelProps) {
  const entries = Object.entries(run.outputs ?? {});
  const [active, setActive] = useState(0);
  useEffect(() => setActive(0), [run.id]);
  if (entries.length === 0) return null;
  const idx = Math.min(active, entries.length - 1);
  const [name, value] = entries[idx];
  return (
    <>
      {entries.length > 1 && (
        <nav className="tabs">
          {entries.map(([n], k) => (
            <button key={n} className={`tab ${k === idx ? "active" : ""}`} onClick={() => setActive(k)}>
              {outputs[n]?.label ?? n}
            </button>
          ))}
        </nav>
      )}
      <Artifact key={`${run.id}:${name}`} name={name} spec={outputs[name]} value={value} steps={steps} busy={busy} onEdit={onEdit} />
    </>
  );
}
