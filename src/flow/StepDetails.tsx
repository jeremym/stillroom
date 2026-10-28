import { fmtMs } from "../api";
import { Glyph, inputGlyph, outputGlyph, stepGlyph } from "../brand";
import { Help, fmtElapsed, useElapsed } from "@overtone-foundry/app-kit";
import { pickViewer } from "../viewers";
import { whereWord } from "../models";
import { KIND_EXPLAINED, KIND_LABEL, STATUS_WORD, describeRef, findStep, parentOf, recordStatus, recordsFor, sourceStepOf, summarize } from "./graph";
import type { InstalledModel, RecipeInfo, Run, StepInfo, StepRecord } from "../types";

/* The side panel for whatever is selected on the flow: a step, an input, or a result. */

interface Props {
  recipe: RecipeInfo;
  run: Run | null;
  records: StepRecord[];
  models: InstalledModel[];
  pins: Record<string, string>;
  onPin: (slot: string, modelId: string | null) => void;
  selectedId: string | null;
  onClose: () => void;
}

export function StepDetails(props: Props) {
  const { selectedId } = props;
  if (!selectedId) return null;
  const [role, rest] = [selectedId.slice(0, selectedId.indexOf(":")), selectedId.slice(selectedId.indexOf(":") + 1)];
  return (
    <aside className="flow-details">
      <button className="link flow-close" onClick={props.onClose}>close</button>
      {role === "in" && <InputDetails name={rest} {...props} />}
      {role === "out" && <OutputDetails name={rest} {...props} />}
      {role === "step" && <StepBody path={rest} {...props} />}
    </aside>
  );
}

function InputDetails({ name, recipe, run }: Props & { name: string }) {
  const spec = recipe.inputs[name];
  if (!spec) return null;
  const value = run?.inputs?.[name];
  return (
    <>
      <h3 className="inspect-head"><Glyph name={inputGlyph(spec.type)} /> {spec.label ?? name} <span className="muted">(input)</span></h3>
      {spec.description && <p>{spec.description}</p>}
      <p className="muted">Type: {spec.type}{spec.required === false ? " · optional" : ""}</p>
      {run && <p><strong>This run:</strong> <span className="mono">{value == null || value === "" ? "not given" : typeof value === "string" ? value : JSON.stringify(value)}</span></p>}
      <p className="muted">Read by: {recipe.steps.filter((s) => s.usesInputs?.includes(name)).map((s) => s.name ?? s.id).join(", ") || "nothing"}</p>
    </>
  );
}

function OutputDetails({ name, recipe, run }: Props & { name: string }) {
  const spec = recipe.outputs[name];
  if (!spec) return null;
  const src = sourceStepOf(spec);
  const value = run?.outputs?.[name];
  return (
    <>
      <h3 className="inspect-head"><Glyph name={outputGlyph(spec.type)} /> {spec.label ?? name} <span className="muted">(result · {spec.type})</span></h3>
      {spec.description && <p>{spec.description}</p>}
      <p className="muted">Comes from {describeRef(spec.from, recipe)}{src ? "" : ""}.</p>
      {value != null ? (
        <details className="flow-output">
          <summary>Preview</summary>
          <div className="flow-preview paper">{pickViewer({ kind: spec.type, value })}</div>
        </details>
      ) : (
        <p className="muted">{run ? "Not produced in this run." : "Run the recipe to produce it."}</p>
      )}
    </>
  );
}

function StepBody({ path, recipe, run, records, models, pins, onPin }: Props & { path: string }) {
  const [mapId, innerId] = path.includes("/") ? path.split("/") : [undefined, path];
  const step = findStep(recipe.steps, innerId);
  if (!step) return null;
  const map = mapId ? findStep(recipe.steps, mapId) : parentOf(recipe.steps, innerId);
  const recs = recordsFor(records, step.id, map?.id);
  const mapRec = map ? recordsFor(records, map.id)[0] : undefined;
  const loopReused = recs.length === 0 && mapRec?.reusedFrom;
  const slot = step.slot ? recipe.modelSlots[step.slot] : undefined;
  const candidates = slot ? models.filter((m) => slot.requires.every((c) => m.capabilities.includes(c))) : [];
  const resolution = step.slot ? run?.resolutions?.[step.slot] : undefined;
  const siblings = map?.steps ?? recipe.steps;
  const feeds = siblings.filter((s) => s.dependsOn.includes(step.id)).map((s) => s.name ?? s.id);
  const feedsOutputs = Object.entries(recipe.outputs).filter(([, o]) => sourceStepOf(o) === step.id).map(([k, o]) => o.label ?? k);
  const readsSteps = step.dependsOn.map((d) => findStep(recipe.steps, d)?.name ?? d);
  const readsInputs = (step.usesInputs ?? []).map((i) => recipe.inputs[i]?.label ?? i);
  const outputKind = Object.values(recipe.outputs).find((o) => sourceStepOf(o) === step.id)?.type ?? "json";

  return (
    <>
      <h3 className="inspect-head"><Glyph name={stepGlyph(step.kind)} /> {step.name ?? step.id} <span className="muted">({KIND_LABEL[step.kind] ?? step.kind}{slot ? ` · ${whereWord(resolution?.modelId ?? (step.slot ? pins[step.slot] : undefined))}` : ""})</span></h3>
      <p>{KIND_EXPLAINED[step.kind] ?? ""}{map ? ` Runs once for each ${map.itemName ?? "item"} in “${map.name ?? map.id}”.` : ""}</p>

      <div className="facts inspect-facts">
        {step.tool && <><span className="label">Program</span><span className="mono">{step.tool}</span></>}
        {step.kind === "agent" && <><span className="label">May call</span><span className="mono">{step.allowedTools?.join(", ") || "no tools"}</span></>}
        {step.when && <><span className="label">Only if</span><span>{describeRef(step.when, recipe, map?.itemName ?? undefined)} is set</span></>}
        <span className="label">Reads</span>
        <span>{readsInputs.length + readsSteps.length === 0
          ? <span className="muted">{map ? `each ${map.itemName ?? "item"}` : "nothing"}</span>
          : <>{readsInputs.map((r) => <span key={`i:${r}`} className="fact"><Glyph name="in" /> {r}</span>)}{readsSteps.map((r) => <span key={`s:${r}`} className="fact">{r}</span>)}</>}</span>
        <span className="label">Feeds</span>
        <span>{[...feeds, ...feedsOutputs].length ? <>{feeds.map((f) => <span key={`f:${f}`} className="fact">{f}</span>)}{feedsOutputs.map((f) => <span key={`o:${f}`} className="fact"><Glyph name={outputGlyph(outputKind)} /> {f}</span>)}</> : <span className="muted">nothing else</span>}</span>
      </div>

      {slot && step.slot && (
        <div className="field">
          <label>
            <strong>Model</strong>
            <span className="muted"> · slot “{step.slot}” needs {slot.requires.join(", ")}</span>
          </label>
          <select value={pins[step.slot] ?? ""} onChange={(e) => onPin(step.slot!, e.target.value || null)}>
            <option value="">Auto{slot.recommended ? ` (recommended ${slot.recommended})` : ""}</option>
            {candidates.map((m) => (
              <option key={m.id} value={m.id}>{m.name}{m.parameterSize ? ` · ${m.parameterSize}` : ""}</option>
            ))}
          </select>
          {resolution && <p className="muted">This run used <span className="mono">{resolution.modelId}</span> ({resolution.via}).</p>}
          <p className="muted">A pin applies to every step on this slot, from the next run on.</p>
        </div>
      )}


      {recs.length === 1 && <SingleRun r={recs[0]} kind={outputKind} />}
      {recs.length > 1 && <ManyRuns recs={recs} map={map} />}
      {loopReused && <p className="muted">Reused: the whole loop kept its earlier results, so this step did not run again for any {map?.itemName ?? "item"}.</p>}
      {recs.length === 0 && !loopReused && run && <p className="muted">Did not run this time.</p>}
    </>
  );
}

function SingleRun({ r, kind }: { r: StepRecord; kind: string }) {
  const status = recordStatus(r);
  const elapsed = useElapsed(status === "running" ? r.startedAt : null);
  const isModel = r.kind === "model" || r.kind === "agent";
  return (
    <div className="flow-run">
      <h4>In this run</h4>
      <p>
        <strong>{STATUS_WORD[status]}</strong>
        {status === "running" && elapsed != null ? ` · ${fmtElapsed(elapsed)} so far` : ""}
        {status === "running" && isModel && (
          <Help title="Why can an AI step take a while?">
            <p>This step is a prompt to <span className="mono">{r.model ?? "a local model"}</span>, running in Ollama on this Mac. Nothing leaves the machine.</p>
            <p>The first call after a pause includes loading the model into memory, which can take ten seconds to a minute on its own. After that, speed depends on the graphics chip and the size of the model and of the text it is reading.</p>
            <p>Long inputs and "thinking" models take longer. You can pin a smaller model on this slot from Set up.</p>
          </Help>
        )}
        {r.durationMs != null && r.durationMs > 0 && status !== "reused" ? ` · ${fmtMs(r.durationMs)}` : ""}
        {r.promptTokens != null ? ` · ${r.promptTokens} → ${r.completionTokens ?? 0} tokens` : ""}
        {r.skipReason && status !== "needs_you" ? ` · ${r.skipReason}` : ""}
      </p>
      {status === "needs_you" && <p>Your decision is waiting in the review panel below.</p>}
      {r.model && <p className="muted mono">{r.model}{r.promptPath ? ` · ${r.promptPath}` : ""}</p>}
      {r.error && <pre className="error">{r.error}</pre>}
      {r.toolCalls && r.toolCalls.length > 0 && (
        <details className="tool-calls">
          <summary>{r.toolCalls.length} tool call{r.toolCalls.length === 1 ? "" : "s"}</summary>
          <ol>
            {r.toolCalls.map((c, i) => (
              <li key={i}>
                <span className="mono">{c.tool}</span> <span className="muted">({fmtMs(c.durationMs)})</span>
                <pre>{JSON.stringify(c.inputs)}</pre>
                {c.error ? <pre className="error">{c.error}</pre> : <pre className="muted">{JSON.stringify(c.output)?.slice(0, 300)}</pre>}
              </li>
            ))}
          </ol>
        </details>
      )}
      {r.output != null && (
        <details className="flow-output">
          <summary>Output</summary>
          <div className="flow-preview paper">{pickViewer({ kind, value: r.output })}</div>
        </details>
      )}
    </div>
  );
}

function ManyRuns({ recs, map }: { recs: StepRecord[]; map: StepInfo | undefined }) {
  const { text } = summarize(recs, false);
  return (
    <div className="flow-run">
      <h4>In this run <span className="muted">· {text}</span></h4>
      <table>
        <thead><tr><th>{map?.itemName ?? "item"}</th><th>Status</th><th>Time</th><th>Output</th></tr></thead>
        <tbody>
          {recs.map((r) => {
            const idx = r.stepId.match(/\[(\d+)\]/)?.[1];
            const status = recordStatus(r);
            return (
              <tr key={r.stepId} className={r.status}>
                <td>{idx != null ? Number(idx) + 1 : ""}</td>
                <td>{STATUS_WORD[status]}{status === "skipped" && r.skipReason ? ` · ${r.skipReason}` : ""}</td>
                <td>{fmtMs(r.durationMs)}</td>
                <td className="flow-cell">{r.error ? <span className="error">{r.error}</span> : shortValue(r.output)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function shortValue(v: unknown): string {
  if (v == null) return "";
  const s = typeof v === "string" ? v : JSON.stringify(v);
  return s.length > 140 ? s.slice(0, 140) + "…" : s;
}
