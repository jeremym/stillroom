import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, fmtMs, fmtTokens } from "../api";
import type { InputSpec, InstalledModel, RecipeInfo, RunEstimate, StepInfo } from "../types";

function stepLabel(s: StepInfo): string {
  const name = s.name ?? s.id;
  if (s.kind === "map" && s.steps?.length) return `${name} (for each: ${s.steps.map(stepLabel).join(" → ")})`;
  if (s.kind === "agent") return `${name} (agent with ${s.allowedTools?.join(", ") || "no tools"})`;
  if (s.kind === "review") return `${name} (you approve)`;
  return name;
}

function describePermissions(recipe: RecipeInfo): string {
  const fs = recipe.permissions?.filesystem;
  const label = (e: string) => {
    const m = e.match(/^\$\{inputs\.([A-Za-z0-9_]+)\}$/);
    if (m) return recipe.inputs[m[1]]?.label ?? m[1];
    if (e === "input") return "the folders and files you choose";
    if (e === "output") return "the run's own folder";
    return e;
  };
  const read = (fs?.read ?? ["input"]).map(label);
  const write = (fs?.write ?? []).map(label);
  const parts = [`reads ${read.join(", ") || "nothing"}`, `writes ${write.length ? write.join(", ") : "only its own results"}`];
  if (recipe.permissions?.shell) parts.push("may run programs");
  if (recipe.permissions?.network?.length) parts.push(`may reach ${recipe.permissions.network.join(", ")}`);
  return parts.join(" · ");
}

interface Props {
  recipes: RecipeInfo[];
  recipe: RecipeInfo;
  onSelect: (recipeId: string) => void;
  /** Model pins for this recipe, shared with the flow view (owned by App, saved per recipe). */
  pins: Record<string, string>;
  onPins: (update: (pins: Record<string, string>) => Record<string, string>) => void;
  models: InstalledModel[];
  running: boolean;
  onRun: (recipe: RecipeInfo, inputs: Record<string, unknown>, pins: Record<string, string>, reuse: boolean) => void;
}

function InputField({
  name, spec, value, onChange,
}: { name: string; spec: InputSpec; value: unknown; onChange: (v: unknown) => void }) {
  const label = spec.label ?? name;
  const optional = spec.required === false;
  async function pick() {
    const picked = await open({
      multiple: spec.type === "files",
      directory: spec.type === "directory",
      filters: spec.accept?.length
        ? [{ name: "Accepted", extensions: spec.accept.map((a) => a.replace(/^\./, "")) }]
        : undefined,
    });
    if (picked) onChange(picked);
  }
  return (
    <div className="field">
      <label>
        <strong>{label}</strong>
        {optional && <span className="muted"> (optional)</span>}
        {spec.description && <span className="muted"> · {spec.description}</span>}
      </label>
      {spec.type === "file" || spec.type === "files" || spec.type === "directory" ? (
        <div className="row">
          <button onClick={pick}>Choose…</button>
          <span className="mono">{typeof value === "string" ? value : Array.isArray(value) ? value.join(", ") : "nothing selected"}</span>
        </div>
      ) : spec.type === "text" ? (
        <textarea rows={6} value={(value as string) ?? ""} onChange={(e) => onChange(e.target.value)} />
      ) : spec.type === "boolean" ? (
        <input type="checkbox" checked={Boolean(value)} onChange={(e) => onChange(e.target.checked)} />
      ) : spec.type === "number" ? (
        <input type="number" value={(value as number) ?? ""} onChange={(e) => onChange(e.target.value === "" ? undefined : Number(e.target.value))} />
      ) : (
        <input type="text" value={(value as string) ?? ""} onChange={(e) => onChange(e.target.value || undefined)} />
      )}
    </div>
  );
}

export function RecipePanel({ recipes, recipe, onSelect, pins, onPins, models, running, onRun }: Props) {
  const [inputs, setInputs] = useState<Record<string, unknown>>({});
  const [estimate, setEstimate] = useState<RunEstimate | null>(null);
  const [reuse, setReuse] = useState(true);

  useEffect(() => {
    setInputs({});
    api.estimate(recipe.id).then(setEstimate).catch(() => setEstimate(null));
  }, [recipe.id]);

  const missing = Object.entries(recipe.inputs)
    .filter(([k, spec]) => spec.required !== false && (inputs[k] == null || inputs[k] === ""))
    .map(([k]) => k);
  const slotCandidates = (requires: string[]) =>
    models.filter((m) => requires.every((c) => m.capabilities.includes(c as never)));
  const unfillable = Object.entries(recipe.modelSlots).filter(([, s]) => slotCandidates(s.requires).length === 0);

  return (
    <section className="panel">
      <div className="row space">
        <h2 style={{ margin: 0 }}>
          <select value={recipe.id} onChange={(e) => onSelect(e.target.value)} className="title-select">
            {recipes.map((r) => (
              <option key={r.id} value={r.id}>{r.name}</option>
            ))}
          </select>
          <span className="muted"> (recipe)</span>
        </h2>
        <span className="muted mono">{recipe.source} · v{recipe.version ?? "0"}</span>
      </div>
      {recipe.description && <p className="muted">{recipe.description}</p>}

      {Object.entries(recipe.inputs).map(([name, spec]) => (
        <InputField key={name} name={name} spec={spec} value={inputs[name]}
          onChange={(v) => setInputs((i) => ({ ...i, [name]: v }))} />
      ))}

      <h3>Models for this run <span className="muted">(model slots)</span></h3>
      {Object.entries(recipe.modelSlots).map(([slot, spec]) => {
        const candidates = slotCandidates(spec.requires);
        return (
          <div className="field" key={slot}>
            <label>
              <strong>{slot}</strong>
              <span className="muted"> · needs {spec.requires.join(", ")}{spec.description ? ` · ${spec.description}` : ""}</span>
            </label>
            <select
              value={pins[slot] ?? ""}
              onChange={(e) => {
                const v = e.target.value;
                onPins((p) => {
                  const next = { ...p };
                  if (v) next[slot] = v; else delete next[slot];
                  return next;
                });
              }}
            >
              <option value="">Auto (prefer {spec.preference ?? "accuracy"}{spec.recommended ? `, recommended ${spec.recommended}` : ""})</option>
              {candidates.map((m) => (
                <option key={m.id} value={m.id}>{m.name}{m.parameterSize ? ` · ${m.parameterSize}` : ""}</option>
              ))}
            </select>
          </div>
        );
      })}

      <p className="muted">Permissions: {describePermissions(recipe)}. Nothing else on disk is touched.</p>
      <p className="muted">
        Steps: {recipe.steps.map(stepLabel).join(" → ")}
        {estimate
          ? ` · last ${estimate.basedOnRuns} run${estimate.basedOnRuns === 1 ? "" : "s"} averaged ${fmtTokens(estimate.avgPromptTokens + estimate.avgCompletionTokens)} tokens, ${fmtMs(estimate.avgDurationMs)}`
          : " · no previous runs to estimate from"}
      </p>

      <label className="row" style={{ margin: "6px 0" }}>
        <input type="checkbox" checked={reuse} onChange={(e) => setReuse(e.target.checked)} />
        <span>Reuse unchanged steps <span className="muted">(selective rerun: a step runs again only if its inputs, prompt, or model changed)</span></span>
      </label>
      <button className="primary" disabled={running || missing.length > 0 || unfillable.length > 0}
        onClick={() => onRun(recipe, inputs, pins, reuse)}>
        {running ? "Running…" : "Run"}
      </button>
      {missing.length > 0 && <span className="muted"> needs: {missing.join(", ")}</span>}
      {unfillable.length > 0 && (
        <span className="muted"> no installed model can fill: {unfillable.map(([s]) => s).join(", ")}</span>
      )}
    </section>
  );
}
