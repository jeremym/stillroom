import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { asDiagram } from "./DiagramView";
import { isDeck, deckToHtml, deckToPptx } from "./SlidesView";
import { isAudio } from "./AudioView";
import { ArtifactEditor } from "./ArtifactEditor";
import { pickViewer } from "../viewers";
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

interface ExportOption { label: string; ext: string; build: () => Promise<string | Uint8Array | { copyFrom: string }>; }

interface ArtifactProps {
  name: string;
  spec: OutputSpec | undefined;
  value: unknown;
  steps: StepInfo[];
  busy: boolean;
  onEdit?: (stepId: string, output: unknown) => void;
}

/** Media produced by tools from files cannot be edited in place; edit what made them. */
function kindIsEditable(kind: string | undefined, value: unknown) {
  if (kind === "audio" || kind === "image" || kind === "video" || kind === "file" || kind === "files") return false;
  return typeof value === "string" || (typeof value === "object" && value !== null);
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
      ? [{ label: svg ? "Save SVG…" : "Save Mermaid…", ext: svg ? "svg" : "mmd", build: async () => svg ?? diagram.source }]
      : kind === "table" && Array.isArray(value)
        ? [{ label: "Save CSV…", ext: "csv", build: async () => toCsv(value as Record<string, unknown>[]) }, { label: "Save JSON…", ext: "json", build: async () => text }]
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
          {value != null && exports.map((opt) => (
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
      ) : value == null ? (
        <p className="muted">Not produced yet. This comes after the review above.</p>
      ) : (
        pickViewer({ kind, value, onSvg: setSvg })
      )}
    </section>
  );
}

function toCsv(rows: Record<string, unknown>[]) {
  const cols = Array.from(new Set(rows.flatMap((r) => Object.keys(r))));
  const cell = (v: unknown) => {
    const s = v == null ? "" : typeof v === "object" ? JSON.stringify(v) : String(v);
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  return [cols.join(","), ...rows.map((r) => cols.map((c) => cell(r[c])).join(","))].join("\n");
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
          {entries.map(([n, v], k) => (
            <button key={n} className={`tab ${k === idx ? "active" : ""} ${v == null ? "pending" : ""}`} onClick={() => setActive(k)}>
              {outputs[n]?.label ?? n}
            </button>
          ))}
        </nav>
      )}
      <Artifact key={`${run.id}:${name}`} name={name} spec={outputs[name]} value={value} steps={steps} busy={busy} onEdit={onEdit} />
    </>
  );
}
