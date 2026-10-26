import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, fmtMs, fmtTokens } from "../api";
import type { InputSpec, InstalledModel, RecipeInfo, RunEstimate } from "../types";

interface Props {
  recipes: RecipeInfo[];
  models: InstalledModel[];
  running: boolean;
  onRun: (recipe: RecipeInfo, inputs: Record<string, unknown>, pins: Record<string, string>) => void;
}

function pinKey(id: string) {
  return `stillroom.pins.${id}`;
}
function loadPins(id: string): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(pinKey(id)) ?? "{}");
  } catch {
    return {};
  }
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

export function RecipePanel({ recipes, models, running, onRun }: Props) {
  const [recipeId, setRecipeId] = useState<string>(recipes[0]?.id ?? "");
  const recipe = recipes.find((r) => r.id === recipeId) ?? recipes[0];
  const [inputs, setInputs] = useState<Record<string, unknown>>({});
  const [pins, setPins] = useState<Record<string, string>>({});
  const [estimate, setEstimate] = useState<RunEstimate | null>(null);

  useEffect(() => {
    if (!recipe) return;
    setInputs({});
    setPins(loadPins(recipe.id));
    api.estimate(recipe.id).then(setEstimate).catch(() => setEstimate(null));
  }, [recipe?.id]);

  useEffect(() => {
    if (!recipe) return;
    try {
      localStorage.setItem(pinKey(recipe.id), JSON.stringify(pins));
    } catch { /* per-viewer convenience only */ }
  }, [pins, recipe?.id]);

  if (!recipe) return <section className="panel"><p className="muted">No recipes found.</p></section>;

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
          <select value={recipe.id} onChange={(e) => setRecipeId(e.target.value)} className="title-select">
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
                setPins((p) => {
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

      <p className="muted">
        Steps: {recipe.steps.map((s) => s.name ?? s.id).join(" → ")}
        {estimate
          ? ` · last ${estimate.basedOnRuns} run${estimate.basedOnRuns === 1 ? "" : "s"} averaged ${fmtTokens(estimate.avgPromptTokens + estimate.avgCompletionTokens)} tokens, ${fmtMs(estimate.avgDurationMs)}`
          : " · no previous runs to estimate from"}
      </p>

      <button className="primary" disabled={running || missing.length > 0 || unfillable.length > 0}
        onClick={() => onRun(recipe, inputs, pins)}>
        {running ? "Running…" : "Run"}
      </button>
      {missing.length > 0 && <span className="muted"> needs: {missing.join(", ")}</span>}
      {unfillable.length > 0 && (
        <span className="muted"> no installed model can fill: {unfillable.map(([s]) => s).join(", ")}</span>
      )}
    </section>
  );
}
