import { useEffect, useState } from "react";
import type { Edge } from "@xyflow/react";
import {
  addStep, dependentsOf, locate, mutate, newInput, newOutput, refFor, refsIn, renameInput, renameKey, renameSlot, renameStepId,
  sourcesFor, stepPathOf, updateStep, STEP_KINDS, type Source, type StepKind,
} from "./draft";
import { KIND_EXPLAINED, KIND_LABEL } from "./graph";
import { fsEntryLabel, netEntryLabel } from "../permissions";
import type { Capability, InputSpec, ModelSlot, OutputSpec, RecipeDoc, RecipeSource, StepDoc, ToolInfo } from "../types";

/* The forms behind Flow View's edit mode. Every change goes up as a whole new document (and,
   for prompts and schemas, a new files map); nothing is saved until the save dialog. */

export const CAPABILITIES: Capability[] = ["text", "structured_output", "tool_calling", "thinking", "vision", "embedding", "text_to_speech", "speech_to_text", "image_generation"];
const INPUT_TYPES: InputSpec["type"][] = ["string", "text", "number", "boolean", "file", "files", "url", "urls", "directory", "json"];
const OUTPUT_TYPES = ["markdown", "text", "json", "html", "slides", "diagram", "image", "audio", "video", "file", "files", "table"];
const PREFERENCES = ["accuracy", "speed", "cost", "local", "quality"];

export type Change = (doc: RecipeDoc, files?: Record<string, string>) => void;

/** A text field whose value is committed on blur or Enter, for keys and ids that other things point at. */
function KeyInput({ value, onCommit, pattern = /^[A-Za-z_][A-Za-z0-9_]*$/ }: { value: string; onCommit: (v: string) => void; pattern?: RegExp }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    if (draft !== value && pattern.test(draft)) onCommit(draft);
    else setDraft(value);
  };
  return (
    <input type="text" className="mono" value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={commit}
      onKeyDown={(e) => { if (e.key === "Enter") (e.target as HTMLInputElement).blur(); }} />
  );
}

/** A reference: typed by hand or picked from what the step can see. */
export function RefField({ value, sources, onChange, allowEmpty = true }: { value: string; sources: Source[]; onChange: (v: string) => void; allowEmpty?: boolean }) {
  return (
    <div className="row ref-field">
      <input type="text" className="mono" value={value} placeholder="${…}" onChange={(e) => onChange(e.target.value)} />
      <select value="" onChange={(e) => { if (e.target.value) onChange(e.target.value); }}>
        <option value="">pick…</option>
        {sources.map((s) => <option key={s.ref} value={s.ref}>{s.label}</option>)}
      </select>
      {allowEmpty && value && <button className="link" onClick={() => onChange("")}>clear</button>}
    </div>
  );
}

function showValue(v: unknown): string {
  return typeof v === "string" ? v : JSON.stringify(v) ?? "";
}
function readValue(text: string): unknown {
  if (text.startsWith("${")) return text;
  try {
    const parsed = JSON.parse(text);
    return typeof parsed === "string" ? text : parsed;
  } catch {
    return text;
  }
}

/** The inputs a step hands to its tool or prompt: key, value or reference. */
function InputsEditor({ inputs, sources, onChange, hints }: { inputs: Record<string, unknown>; sources: Source[]; onChange: (i: Record<string, unknown>) => void; hints?: ToolInfo["parameters"] }) {
  const expected = Object.keys(hints?.properties ?? {});
  const missing = expected.filter((k) => !(k in inputs));
  return (
    <div className="field">
      <label><strong>Inputs</strong> <span className="muted">· what this step is handed, by name</span></label>
      {Object.entries(inputs).map(([k, v]) => (
        <div className="inputs-row" key={k}>
          <KeyInput value={k} onCommit={(nk) => onChange(renameKey(inputs, k, nk))} />
          <span className="muted">=</span>
          <input type="text" className="mono" value={showValue(v)} onChange={(e) => onChange({ ...inputs, [k]: readValue(e.target.value) })} />
          <div className="inputs-row-tools">
            {hints?.properties?.[k]?.description && <span className="muted hint">{hints.properties[k].description}</span>}
            <select value="" onChange={(e) => { if (e.target.value) onChange({ ...inputs, [k]: e.target.value }); }}>
              <option value="">pick a source…</option>
              {sources.map((s) => <option key={s.ref} value={s.ref}>{s.label}</option>)}
            </select>
            <button className="link" onClick={() => { const next = { ...inputs }; delete next[k]; onChange(next); }}>remove</button>
          </div>
        </div>
      ))}
      <div className="row">
        <button onClick={() => onChange({ ...inputs, [`input${Object.keys(inputs).length + 1}`]: "" })}>Add input</button>
        {missing.length > 0 && (
          <button onClick={() => onChange({ ...inputs, ...Object.fromEntries(missing.map((k) => [k, ""])) })}>
            Add the ones the tool expects ({missing.join(", ")})
          </button>
        )}
      </div>
    </div>
  );
}

interface StepFormProps {
  doc: RecipeDoc;
  files: Record<string, string>;
  path: string;
  tools: ToolInfo[];
  onChange: Change;
  onRemove: () => void;
  onSelect: (path: string) => void;
}

export function StepForm({ doc, files, path, tools, onChange, onRemove, onSelect }: StepFormProps) {
  const loc = locate(doc, path);
  const [confirmRemove, setConfirmRemove] = useState(false);
  useEffect(() => setConfirmRemove(false), [path]);
  if (!loc) return null;
  const s = loc.step;
  const sources = sourcesFor(doc, path);
  const set = (patch: Partial<StepDoc>) => onChange(updateStep(doc, path, patch));
  const deps = dependentsOf(doc, path);
  const tool = tools.find((t) => t.id === s.tool);
  const slots = Object.keys(doc.modelSlots ?? {});
  const hasPrompt = s.kind === "model" || s.kind === "agent";
  const promptPath = s.prompt ?? `prompts/${s.id}.md`;

  return (
    <>
      <h3>{s.name ?? s.id} <span className="muted">({KIND_LABEL[s.kind] ?? s.kind})</span></h3>
      <p className="muted">{KIND_EXPLAINED[s.kind]}</p>

      <div className="field two">
        <label><strong>Name</strong><input type="text" value={s.name ?? ""} onChange={(e) => set({ name: e.target.value || undefined })} /></label>
        <label><strong>Id</strong> <span className="muted">(what other steps call it)</span>
          <KeyInput value={s.id} onCommit={(id) => {
            const d = renameStepId(doc, path, id);
            if (d !== doc) {
              onChange(d);
              onSelect(loc.parent ? `${loc.parent.id}/${id}` : id);
            }
          }} />
        </label>
      </div>

      {s.kind === "tool" && (
        <div className="field">
          <label><strong>Tool</strong></label>
          <select value={s.tool ?? ""} onChange={(e) => set({ tool: e.target.value })}>
            <option value="">choose a tool…</option>
            {tools.map((t) => <option key={t.id} value={t.id}>{t.id}{t.description ? ` · ${t.description.slice(0, 60)}` : ""}</option>)}
          </select>
          {tool?.description && <p className="muted">{tool.description}</p>}
          {tool?.parameters?.properties && (
            <p className="muted">Expects: {Object.entries(tool.parameters.properties).map(([k, p]) => `${k}${tool.parameters?.required?.includes(k) ? "" : "?"}${p.type ? ` (${p.type})` : ""}`).join(", ")}</p>
          )}
        </div>
      )}

      {hasPrompt && (
        <>
          <div className="field">
            <label><strong>Model slot</strong> <span className="muted">· which kind of model answers</span></label>
            <select value={s.slot ?? ""} onChange={(e) => set({ slot: e.target.value })}>
              <option value="">choose…</option>
              {slots.map((k) => <option key={k} value={k}>{k} · needs {(doc.modelSlots?.[k].requires ?? []).join(", ")}</option>)}
            </select>
          </div>
          <div className="field">
            <label><strong>Prompt</strong> <span className="muted mono">· {promptPath}</span></label>
            <textarea rows={10} value={files[promptPath] ?? ""} placeholder="What the model is asked to do. Inputs are given to it as JSON after this text."
              onChange={(e) => onChange(s.prompt ? doc : updateStep(doc, path, { prompt: promptPath }), { ...files, [promptPath]: e.target.value })} />
          </div>
          <SchemaField doc={doc} files={files} path={path} onChange={onChange} />
          <ParamsField step={s} set={set} />
        </>
      )}

      {s.kind === "agent" && (
        <div className="field">
          <label><strong>May call</strong> <span className="muted">· tools the agent is allowed to use</span></label>
          {tools.map((t) => (
            <label className="check" key={t.id} title={t.parameters ? t.description : "This tool cannot describe itself to a model"}>
              <input type="checkbox" disabled={!t.parameters} checked={s.allowedTools?.includes(t.id) ?? false}
                onChange={(e) => set({ allowedTools: e.target.checked ? [...(s.allowedTools ?? []), t.id] : (s.allowedTools ?? []).filter((x) => x !== t.id) })} />
              <span className="mono">{t.id}</span> <span className="muted">{t.description}</span>
            </label>
          ))}
          <div className="two">
            <label><strong>Max turns</strong><input type="number" min={1} value={s.maxTurns ?? 10} onChange={(e) => set({ maxTurns: Number(e.target.value) || 10 })} /></label>
            <label><strong>Max tool calls</strong><input type="number" min={0} value={s.maxToolCalls ?? 20} onChange={(e) => set({ maxToolCalls: Number(e.target.value) || 0 })} /></label>
          </div>
        </div>
      )}

      {s.kind === "map" && (
        <>
          <div className="field">
            <label><strong>Loop over</strong> <span className="muted">· a list; the inner steps run once per item</span></label>
            <RefField value={s.over ?? ""} sources={sources} onChange={(v) => set({ over: v })} allowEmpty={false} />
          </div>
          <div className="two">
            <label><strong>Call each item</strong><KeyInput value={s.as ?? "item"} onCommit={(v) => set({ as: v })} /></label>
            <label><strong>At a time</strong><input type="number" min={1} max={32} value={s.concurrency ?? 4} onChange={(e) => set({ concurrency: Number(e.target.value) || 4 })} /></label>
          </div>
          <div className="field">
            <label><strong>Inner steps</strong></label>
            <ol className="inner-steps">
              {(s.steps ?? []).map((i) => <li key={i.id}><button className="link" onClick={() => onSelect(`${s.id}/${i.id}`)}>{i.name ?? i.id}</button> <span className="muted">{KIND_LABEL[i.kind]}</span></li>)}
            </ol>
            <AddStepSelect label="Add inner step" onPick={(kind) => { const r = addStep(doc, kind, s.id); onChange(r.doc, hasPromptKind(kind) ? withPrompt(files, r) : undefined); onSelect(r.path); }} />
          </div>
        </>
      )}

      {s.kind === "review" && (
        <>
          <div className="field">
            <label><strong>Items to decide on</strong> <span className="muted">· a list; you approve or reject each one</span></label>
            <RefField value={s.items ?? ""} sources={sources} onChange={(v) => set({ items: v })} allowEmpty={false} />
          </div>
          <div className="field">
            <label><strong>Title</strong> <span className="muted">· what you are deciding</span><input type="text" value={s.title ?? ""} onChange={(e) => set({ title: e.target.value || undefined })} /></label>
          </div>
          <div className="field">
            <label><strong>Show</strong> <span className="muted">· paths inside each item, first one as the headline, comma separated</span>
              <input type="text" className="mono" value={(s.show ?? []).join(", ")} onChange={(e) => set({ show: e.target.value.split(",").map((x) => x.trim()).filter(Boolean) })} />
            </label>
          </div>
        </>
      )}

      {s.kind !== "review" && (
        <InputsEditor inputs={s.inputs ?? {}} sources={sources} onChange={(inputs) => set({ inputs })} hints={tool?.parameters ?? null} />
      )}

      <div className="field">
        <label><strong>Runs only if</strong> <span className="muted">· optional; a value that must be set</span></label>
        <RefField value={s.when ?? ""} sources={sources} onChange={(v) => set({ when: v || undefined })} />
      </div>
      {s.kind !== "map" && s.kind !== "review" && (
        <label className="check">
          <input type="checkbox" checked={s.checkpoint !== false} onChange={(e) => set({ checkpoint: e.target.checked ? undefined : false })} />
          Reuse this step's result when nothing it depends on changed
        </label>
      )}

      <div className="field danger">
        {deps.length > 0 && <p className="muted">{deps.length} thing{deps.length === 1 ? " reads" : "s read"} from this step ({deps.join(", ")}). Removing it leaves them with problems until you rewire them.</p>}
        {confirmRemove
          ? <div className="row"><button className="danger" onClick={onRemove}>Yes, remove “{s.name ?? s.id}”</button><button onClick={() => setConfirmRemove(false)}>Keep it</button></div>
          : <button onClick={() => setConfirmRemove(true)}>Remove this step…</button>}
      </div>
    </>
  );
}

function hasPromptKind(kind: StepKind) {
  return kind === "model" || kind === "agent";
}

/** A new prompt file for a freshly added model or agent step. */
export function withPrompt(files: Record<string, string>, r: { doc: RecipeDoc; path: string }): Record<string, string> {
  const loc = locate(r.doc, r.path);
  if (!loc?.step.prompt || files[loc.step.prompt]) return files;
  return { ...files, [loc.step.prompt]: "" };
}

export function AddStepSelect({ label, onPick }: { label: string; onPick: (kind: StepKind) => void }) {
  return (
    <select value="" onChange={(e) => { if (e.target.value) onPick(e.target.value as StepKind); }}>
      <option value="">{label}…</option>
      {STEP_KINDS.map((k) => <option key={k} value={k}>{KIND_LABEL[k]}</option>)}
    </select>
  );
}

function SchemaField({ doc, files, path, onChange }: { doc: RecipeDoc; files: Record<string, string>; path: string; onChange: Change }) {
  const s = locate(doc, path)!.step;
  const mode = s.outputSchema == null ? "none" : typeof s.outputSchema === "string" ? "file" : "inline";
  const filePath = typeof s.outputSchema === "string" ? s.outputSchema : `schemas/${s.id}.json`;
  const [inline, setInline] = useState(() => (mode === "inline" ? JSON.stringify(s.outputSchema, null, 2) : ""));
  const [inlineError, setInlineError] = useState<string | null>(null);
  useEffect(() => { if (mode === "inline") setInline(JSON.stringify(s.outputSchema, null, 2)); }, [path]);
  const fileError = mode === "file" && files[filePath] != null ? (() => { try { JSON.parse(files[filePath]); return null; } catch (e) { return String(e); } })() : null;
  return (
    <div className="field">
      <label><strong>Output shape</strong> <span className="muted">· a JSON Schema the answer must fit; needed when other steps read parts of it</span></label>
      <select value={mode} onChange={(e) => {
        const m = e.target.value;
        if (m === "none") onChange(updateStep(doc, path, { outputSchema: undefined }));
        else if (m === "file") onChange(updateStep(doc, path, { outputSchema: filePath }), files[filePath] != null ? files : { ...files, [filePath]: '{\n  "type": "object",\n  "properties": {}\n}\n' });
        else { onChange(updateStep(doc, path, { outputSchema: { type: "object", properties: {} } })); setInline('{\n  "type": "object",\n  "properties": {}\n}'); }
      }}>
        <option value="none">Free text</option>
        <option value="file">Schema file ({filePath})</option>
        <option value="inline">Schema written here</option>
      </select>
      {mode === "file" && <textarea className="mono" rows={8} value={files[filePath] ?? ""} onChange={(e) => onChange(doc, { ...files, [filePath]: e.target.value })} />}
      {mode === "inline" && (
        <textarea className="mono" rows={8} value={inline} onChange={(e) => {
          setInline(e.target.value);
          try { onChange(updateStep(doc, path, { outputSchema: JSON.parse(e.target.value) })); setInlineError(null); } catch (err) { setInlineError(String(err)); }
        }} />
      )}
      {(fileError ?? inlineError) && <pre className="error">{fileError ?? inlineError}</pre>}
    </div>
  );
}

function ParamsField({ step, set }: { step: StepDoc; set: (p: Partial<StepDoc>) => void }) {
  const p = step.params ?? {};
  const update = (patch: Partial<NonNullable<StepDoc["params"]>>) => {
    const next = { ...p, ...patch };
    for (const k of Object.keys(next) as (keyof typeof next)[]) if (next[k] === undefined || next[k] === null) delete next[k];
    set({ params: Object.keys(next).length ? next : undefined });
  };
  return (
    <div className="field">
      <label><strong>Model settings</strong></label>
      <div className="two">
        <label>Thinking
          <select value={p.think === undefined ? "auto" : String(p.think)} onChange={(e) => update({ think: e.target.value === "auto" ? undefined : e.target.value === "true" ? true : e.target.value === "false" ? false : e.target.value })}>
            <option value="auto">model default</option><option value="false">off (faster)</option><option value="true">on</option>
          </select>
        </label>
        <label>Temperature <span className="muted">(0 = steady, 1 = varied)</span>
          <input type="number" step={0.1} min={0} max={2} value={p.temperature ?? ""} onChange={(e) => update({ temperature: e.target.value === "" ? undefined : Number(e.target.value) })} />
        </label>
        <label>Context length <span className="muted">(tokens)</span>
          <input type="number" step={1024} min={512} value={p.contextLength ?? ""} onChange={(e) => update({ contextLength: e.target.value === "" ? undefined : Number(e.target.value) })} />
        </label>
        <label>Max answer <span className="muted">(tokens)</span>
          <input type="number" min={1} value={p.maxTokens ?? ""} onChange={(e) => update({ maxTokens: e.target.value === "" ? undefined : Number(e.target.value) })} />
        </label>
      </div>
    </div>
  );
}

export function InputForm({ doc, name, onChange, onRemove, onSelect }: { doc: RecipeDoc; name: string; onChange: Change; onRemove: () => void; onSelect: (nodeId: string) => void }) {
  const spec = doc.inputs?.[name];
  if (!spec) return null;
  const set = (patch: Partial<InputSpec>) => onChange(mutate(doc, (d) => {
    const next = { ...d.inputs![name], ...patch } as InputSpec;
    for (const k of Object.keys(next) as (keyof InputSpec)[]) if (next[k] === undefined) delete next[k];
    d.inputs![name] = next;
  }));
  const readers = doc.steps.filter((s) => refsIn(s).some((r) => r === `\${inputs.${name}}` || r.startsWith(`\${inputs.${name}.`)));
  const isFile = ["file", "files"].includes(spec.type);
  return (
    <>
      <h3>{spec.label ?? name} <span className="muted">(input)</span></h3>
      <div className="field two">
        <label><strong>Label</strong><input type="text" value={spec.label ?? ""} onChange={(e) => set({ label: e.target.value || undefined })} /></label>
        <label><strong>Name</strong> <span className="muted">(in references)</span>
          <KeyInput value={name} onCommit={(n) => { const d = renameInput(doc, name, n); if (d !== doc) { onChange(d); onSelect(`in:${n}`); } }} />
        </label>
      </div>
      <div className="field two">
        <label><strong>Type</strong>
          <select value={spec.type} onChange={(e) => set({ type: e.target.value as InputSpec["type"] })}>
            {INPUT_TYPES.map((t) => <option key={t} value={t}>{t}</option>)}
          </select>
        </label>
        <label className="check" style={{ alignSelf: "end" }}>
          <input type="checkbox" checked={spec.required !== false} onChange={(e) => set({ required: e.target.checked ? undefined : false })} /> Required
        </label>
      </div>
      <div className="field">
        <label><strong>Description</strong> <span className="muted">· shown under the field</span><textarea rows={2} value={spec.description ?? ""} onChange={(e) => set({ description: e.target.value || undefined })} /></label>
      </div>
      {isFile && (
        <div className="field">
          <label><strong>Accepted extensions</strong> <span className="muted">· comma separated</span>
            <input type="text" className="mono" value={(spec.accept ?? []).join(", ")} onChange={(e) => set({ accept: e.target.value ? e.target.value.split(",").map((x) => x.trim()).filter(Boolean) : undefined })} />
          </label>
        </div>
      )}
      <p className="muted">Read by: {readers.map((s) => s.name ?? s.id).join(", ") || "nothing yet (drag from this input to a step)"}</p>
      <button onClick={onRemove}>Remove this input</button>
    </>
  );
}

export function OutputForm({ doc, name, onChange, onRemove, onSelect }: { doc: RecipeDoc; name: string; onChange: Change; onRemove: () => void; onSelect: (nodeId: string) => void }) {
  const spec = doc.outputs?.[name];
  if (!spec) return null;
  const set = (patch: Partial<OutputSpec>) => onChange(mutate(doc, (d) => {
    const next = { ...d.outputs[name], ...patch } as OutputSpec;
    for (const k of Object.keys(next) as (keyof OutputSpec)[]) if (next[k] === undefined) delete next[k];
    d.outputs[name] = next;
  }));
  return (
    <>
      <h3>{spec.label ?? name} <span className="muted">(result)</span></h3>
      <div className="field two">
        <label><strong>Label</strong><input type="text" value={spec.label ?? ""} onChange={(e) => set({ label: e.target.value || undefined })} /></label>
        <label><strong>Name</strong>
          <KeyInput value={name} onCommit={(n) => { if (!doc.outputs[n]) { onChange(mutate(doc, (d) => { d.outputs = renameKey(d.outputs, name, n); })); onSelect(`out:${n}`); } }} />
        </label>
      </div>
      <div className="field">
        <label><strong>Comes from</strong> <span className="muted">· a step's output, or a part of it</span></label>
        <RefField value={spec.from} sources={sourcesFor(doc, null)} onChange={(v) => set({ from: v })} allowEmpty={false} />
      </div>
      <div className="field">
        <label><strong>Shown as</strong>
          <select value={spec.type} onChange={(e) => set({ type: e.target.value })}>
            {OUTPUT_TYPES.map((t) => <option key={t} value={t}>{t}</option>)}
          </select>
        </label>
      </div>
      <div className="field">
        <label><strong>Description</strong><textarea rows={2} value={spec.description ?? ""} onChange={(e) => set({ description: e.target.value || undefined })} /></label>
      </div>
      <button onClick={onRemove}>Remove this result</button>
    </>
  );
}

export function SlotsEditor({ doc, onChange, onClose }: { doc: RecipeDoc; onChange: Change; onClose: () => void }) {
  const slots = doc.modelSlots ?? {};
  const set = (name: string, patch: Partial<ModelSlot>) => onChange(mutate(doc, (d) => {
    const next = { ...d.modelSlots![name], ...patch } as ModelSlot;
    for (const k of Object.keys(next) as (keyof ModelSlot)[]) if (next[k] === undefined) delete next[k];
    d.modelSlots![name] = next;
  }));
  const users = (name: string) => doc.steps.flatMap((s) => [s, ...(s.steps ?? [])]).filter((s) => s.slot === name).map((s) => s.name ?? s.id);
  return (
    <>
      <h3>Model slots <span className="muted">(which kinds of model the recipe needs)</span></h3>
      <p className="muted">A step names a slot, not a model. At run time each slot is filled by a pinned, recommended, or capable installed model.</p>
      {Object.entries(slots).map(([name, slot]) => (
        <div className="edit-block" key={name}>
          <div className="row space">
            <KeyInput value={name} onCommit={(n) => onChange(renameSlot(doc, name, n))} />
            <button className="link" disabled={users(name).length > 0} title={users(name).length ? `Used by ${users(name).join(", ")}` : ""}
              onClick={() => onChange(mutate(doc, (d) => { delete d.modelSlots![name]; }))}>remove</button>
          </div>
          <div className="caps">
            {CAPABILITIES.map((c) => (
              <label className="check" key={c}>
                <input type="checkbox" checked={slot.requires.includes(c)} onChange={(e) => set(name, { requires: e.target.checked ? [...slot.requires, c] : slot.requires.filter((x) => x !== c) })} /> {c}
              </label>
            ))}
          </div>
          <div className="two">
            <label>Prefer
              <select value={slot.preference ?? "accuracy"} onChange={(e) => set(name, { preference: e.target.value })}>
                {PREFERENCES.map((p) => <option key={p} value={p}>{p}</option>)}
              </select>
            </label>
            <label>Recommended model <span className="muted">(optional id)</span>
              <input type="text" className="mono" value={slot.recommended ?? ""} onChange={(e) => set(name, { recommended: e.target.value || undefined })} />
            </label>
          </div>
          <label>Description<input type="text" value={slot.description ?? ""} onChange={(e) => set(name, { description: e.target.value || undefined })} /></label>
          <p className="muted">Used by: {users(name).join(", ") || "no step yet"}</p>
        </div>
      ))}
      <div className="row">
        <button onClick={() => onChange(mutate(doc, (d) => { (d.modelSlots ??= {})[Object.keys(d.modelSlots).includes("writer") ? `slot${Object.keys(d.modelSlots).length + 1}` : "writer"] = { requires: ["text"], preference: "accuracy" }; }))}>Add slot</button>
        <button onClick={onClose}>Done</button>
      </div>
    </>
  );
}

/** What the recipe may read, write, and reach. Entries are the runtime's shorthands, inputs, or paths and hosts. */
export function PermissionsEditor({ doc, onChange, onClose }: { doc: RecipeDoc; onChange: Change; onClose: () => void }) {
  const inputs = doc.inputs ?? {};
  const perms = doc.permissions ?? {};
  const read = perms.filesystem?.read ?? ["input"];
  const write = perms.filesystem?.write ?? [];
  const net = perms.network ?? [];
  const setFs = (key: "read" | "write", list: string[]) => onChange(mutate(doc, (d) => {
    d.permissions ??= {};
    d.permissions.filesystem ??= {};
    d.permissions.filesystem[key] = list;
  }));
  const setNet = (list: string[]) => onChange(mutate(doc, (d) => {
    d.permissions ??= {};
    if (list.length) d.permissions.network = list; else delete d.permissions.network;
  }));
  const pathInputs = Object.entries(inputs).filter(([, s]) => ["file", "files", "directory"].includes(s.type)).map(([k]) => k);
  const linkInputs = Object.entries(inputs).filter(([, s]) => ["url", "urls"].includes(s.type)).map(([k]) => k);
  const fsChoices = (list: string[]) => [
    ["input", "the files and folders you choose"], ["output", "the run's own folder"],
    ...pathInputs.map((k) => [`\${inputs.${k}}`, inputs[k].label ?? k]),
  ].filter(([v]) => !list.includes(v));
  const netChoices = (list: string[]) => [
    ["input", "the sites in the links you give it"],
    ...linkInputs.map((k) => [`\${inputs.${k}}`, `the sites in “${inputs[k].label ?? k}”`]),
  ].filter(([v]) => !list.includes(v));
  return (
    <>
      <h3>Permissions <span className="muted">(what the recipe may touch)</span></h3>
      <p className="muted">Every file and web tool checks these at run time. The run's own results folder is always writable. Models are governed by your provider settings, not here.</p>
      <EntryList title="May read" entries={read} label={(e) => fsEntryLabel(e, inputs)} choices={fsChoices(read)} custom="a folder path" onChange={(l) => setFs("read", l)} />
      <EntryList title="May write" entries={write} label={(e) => fsEntryLabel(e, inputs)} choices={fsChoices(write)} custom="a folder path" onChange={(l) => setFs("write", l)} />
      <EntryList title="May reach" entries={net} label={(e) => netEntryLabel(e, inputs)} choices={netChoices(net)} custom="a host, like example.com or *.example.com" onChange={setNet} empty="no network" />
      <button onClick={onClose}>Done</button>
    </>
  );
}

function EntryList({ title, entries, label, choices, custom, onChange, empty = "nothing" }: {
  title: string; entries: string[]; label: (e: string) => string; choices: string[][]; custom: string; onChange: (l: string[]) => void; empty?: string;
}) {
  const [text, setText] = useState("");
  return (
    <div className="field">
      <label><strong>{title}</strong></label>
      {entries.length === 0 && <p className="muted">{empty}</p>}
      <ul className="entries">
        {entries.map((e) => (
          <li key={e}>{label(e)} <span className="muted mono">{e !== label(e) ? e : ""}</span> <button className="link" onClick={() => onChange(entries.filter((x) => x !== e))}>remove</button></li>
        ))}
      </ul>
      <div className="row" style={{ margin: "4px 0" }}>
        <select value="" onChange={(e) => { if (e.target.value) onChange([...entries, e.target.value]); }}>
          <option value="">add…</option>
          {choices.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
        </select>
        <input type="text" className="mono" placeholder={custom} value={text} onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && text.trim()) { onChange([...entries, text.trim()]); setText(""); } }} />
        <button disabled={!text.trim()} onClick={() => { onChange([...entries, text.trim()]); setText(""); }}>Add</button>
      </div>
    </div>
  );
}

/** After dragging a connection onto a step: which of its inputs should receive the value? */
export function WirePrompt({ doc, source, target, onConnect, onCancel }: { doc: RecipeDoc; source: string; target: string; onConnect: (key: string) => void; onCancel: () => void }) {
  const path = stepPathOf(target);
  const loc = path ? locate(doc, path) : null;
  const [key, setKey] = useState("");
  const [fresh, setFresh] = useState("");
  if (!loc) return null;
  const s = loc.step;
  const keys = Object.keys(s.inputs ?? {});
  const special: [string, string][] = [];
  if (s.kind === "map") special.push(["over", "the list to loop over"]);
  if (s.kind === "review") special.push(["items", "the items to decide on"]);
  special.push(["when", "the condition it runs under"]);
  const ref = refFor(source) ?? source;
  const chosen = key === "__new" ? fresh.trim() : key;
  return (
    <>
      <h3>Connect</h3>
      <p>Feed <span className="mono">{ref}</span> into <strong>{s.name ?? s.id}</strong> as:</p>
      <select value={key} onChange={(e) => setKey(e.target.value)}>
        <option value="">choose…</option>
        {keys.map((k) => <option key={k} value={k}>input “{k}” (currently {showValue(s.inputs![k]) || "empty"})</option>)}
        {special.map(([k, l]) => <option key={k} value={k}>{l}</option>)}
        {s.kind !== "review" && <option value="__new">a new input…</option>}
      </select>
      {key === "__new" && <input type="text" className="mono" placeholder="name" value={fresh} onChange={(e) => setFresh(e.target.value)} />}
      <div className="row">
        <button className="primary" style={{ marginTop: 0 }} disabled={!chosen || (key === "__new" && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(chosen))} onClick={() => onConnect(chosen)}>Connect</button>
        <button onClick={onCancel}>Cancel</button>
      </div>
    </>
  );
}

/** A selected line on the graph: what it carries, and a way to cut it. */
export function ConnectionPanel({ doc, edge, labelOf, onDisconnect, onClose }: { doc: RecipeDoc; edge: Edge; labelOf: (nodeId: string) => string; onDisconnect: () => void; onClose: () => void }) {
  const ref = refFor(edge.source);
  const base = ref?.slice(0, -1) ?? "";
  const path = stepPathOf(edge.target);
  const loc = path ? locate(doc, path) : null;
  const carried: string[] = [];
  if (loc && ref) {
    const hit = (v: unknown) => typeof v === "string" && (v === ref || v.startsWith(base + "."));
    for (const [k, v] of Object.entries(loc.step.inputs ?? {})) if (hit(v) || refsIn(v).some(hit)) carried.push(`input “${k}” = ${showValue(v)}`);
    if (hit(loc.step.when)) carried.push(`runs only if ${loc.step.when}`);
    if (hit(loc.step.over)) carried.push(`loops over ${loc.step.over}`);
    if (hit(loc.step.items)) carried.push(`decides on ${loc.step.items}`);
  } else if (edge.target.startsWith("out:")) {
    carried.push(`result “${edge.target.slice(4)}” comes from ${doc.outputs?.[edge.target.slice(4)]?.from ?? ""}`);
  }
  return (
    <>
      <h3>Connection</h3>
      <p><strong>{labelOf(edge.source)}</strong> feeds <strong>{labelOf(edge.target)}</strong>.</p>
      {carried.length > 0 && <ul>{carried.map((c) => <li key={c} className="mono">{c}</li>)}</ul>}
      <div className="row">
        <button onClick={onDisconnect}>Disconnect</button>
        <button className="link" onClick={onClose}>close</button>
      </div>
    </>
  );
}

export function SaveDialog({ doc, source, sourceId, busy, error, onSave, onCancel }: {
  doc: RecipeDoc; source: RecipeSource; sourceId: string; busy: boolean; error: string | null;
  onSave: (id: string, name: string, description: string) => void; onCancel: () => void;
}) {
  const builtin = source === "builtin";
  const [id, setId] = useState(builtin ? `${doc.id}-custom` : doc.id);
  const [name, setName] = useState(builtin ? `${doc.name} (custom)` : doc.name);
  const [description, setDescription] = useState(doc.description ?? "");
  const validId = /^[a-z0-9][a-z0-9-]*$/.test(id);
  const replaces = !builtin && id === sourceId;
  const shadows = builtin && id === sourceId;
  return (
    <>
      <h3>Save recipe</h3>
      <div className="field"><label><strong>Name</strong><input type="text" value={name} onChange={(e) => setName(e.target.value)} /></label></div>
      <div className="field"><label><strong>Id</strong> <span className="muted">· folder name, lowercase letters, digits, dashes</span><input type="text" className="mono" value={id} onChange={(e) => setId(e.target.value)} /></label></div>
      <div className="field"><label><strong>Description</strong><textarea rows={3} value={description} onChange={(e) => setDescription(e.target.value)} /></label></div>
      <p className="muted">
        {replaces ? "Replaces your saved copy of this recipe." : shadows ? "Keeps the same id as the built-in recipe, so your copy takes its place in the list. The built-in stays on disk." : "Saved as a new recipe in your recipes folder."}
      </p>
      {error && <pre className="error">{error}</pre>}
      <div className="row">
        <button className="primary" style={{ marginTop: 0 }} disabled={busy || !validId || !name.trim()} onClick={() => onSave(id, name.trim(), description.trim())}>{busy ? "Saving…" : "Save"}</button>
        <button disabled={busy} onClick={onCancel}>Cancel</button>
      </div>
    </>
  );
}

/** New input/result helpers re-exported for the toolbar. */
export { newInput, newOutput };
