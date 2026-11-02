import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, fmtTokens } from "../api";
import { describePermissions } from "../permissions";
import { defaultInputs } from "../recipes/defaults";
import { aboutMs, estimateWord, inputsKnownUpTo, keepHeldWord, keepWord } from "../stages";
import type { InputSpec, InstalledModel, KeepState, RecipeInfo, ResolutionPreview, RunEstimate, StepInfo } from "../types";
import { Glyph } from "../brand";
import { isLocalModel, whereWord } from "../models";
import { allTicked, consentOf } from "../consent";
import { ConsentZone } from "./ConsentZone";

const VIA_WORD: Record<string, string> = {
  pinned: "your pin on this slot",
  default: "your default for this kind of step (Settings › Models)",
  recommended: "the recipe's recommendation",
  capability: "the best installed model, by preference",
};

function stepLabel(s: StepInfo): string {
  const name = s.name ?? s.id;
  if (s.kind === "map" && s.steps?.length) return `${name} (for each: ${s.steps.map(stepLabel).join(" → ")})`;
  if (s.kind === "agent") return `${name} (agent with ${s.allowedTools?.join(", ") || "no tools"})`;
  if (s.kind === "review") return `${name} (you approve)`;
  return name;
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
  /** Watch one of the recipe's folder inputs: the recipe runs by itself when something lands there. */
  onWatch: (recipe: RecipeInfo, input: string, inputs: Record<string, unknown>, pins: Record<string, string>) => void;
  /** The strip above wants to know which stage still needs you. */
  onInputsChange?: (inputs: Record<string, unknown>) => void;
  /** Kept stages' state for the inputs typed so far, by stage id. */
  keeps?: Record<string, KeepState>;
  /** Empty a stage's keep so the next run builds it again. */
  onRebuild?: (stage: string) => void;
  /** The consent step wrote an answer; the recipe list should be read again. The engine also says so
   *  with `recipes:changed`; this is for the plain-browser dev build, where that event never comes. */
  onConsentChanged?: () => void;
}

export function InputField({
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
      ) : spec.type === "urls" ? (
        <textarea rows={4} className="mono" placeholder="one link per line" value={Array.isArray(value) ? value.join("\n") : ""}
          onChange={(e) => { const links = e.target.value.split("\n").map((l) => l.trim()).filter(Boolean); onChange(links.length ? links : undefined); }} />
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

export function RecipePanel({ recipes, recipe, onSelect, pins, onPins, models, running, onRun, onWatch, onInputsChange, keeps, onRebuild, onConsentChanged }: Props) {
  const [confirmRebuild, setConfirmRebuild] = useState<string | null>(null);
  // The consent step: which needed lines are ticked; every one ticked is the answer, written at once.
  const [ticked, setTicked] = useState<Set<string>>(new Set());
  const [consentBusy, setConsentBusy] = useState(false);
  const [consentError, setConsentError] = useState<string | null>(null);
  const consent = consentOf(recipe);
  const permissionNeeded = consent.needed.length > 0;
  const [inputs, setInputs] = useState<Record<string, unknown>>({});
  const [estimate, setEstimate] = useState<RunEstimate | null>(null);
  const [preview, setPreview] = useState<ResolutionPreview | null>(null);
  const [reuse, setReuse] = useState(true);
  const folderInputs = Object.entries(recipe.inputs).filter(([, spec]) => spec.type === "directory").map(([k]) => k);
  const [watchInput, setWatchInput] = useState<string>("");

  useEffect(() => { onInputsChange?.(inputs); }, [inputs, onInputsChange]);

  useEffect(() => {
    setInputs(defaultInputs(recipe.inputs));
    setTicked(new Set());
    setConsentError(null);
    setWatchInput(folderInputs[0] ?? "");
    api.estimate(recipe.id).then(setEstimate).catch(() => setEstimate(null));
  }, [recipe.id]);

  // What Auto would really pick, right now, with these pins and the user's defaults.
  useEffect(() => {
    let live = true;
    api.previewResolution(recipe.id, pins).then((p) => { if (live) setPreview(p); }).catch(() => { if (live) setPreview(null); });
    return () => { live = false; };
  }, [recipe.id, pins, models]);

  // ⌘R from the menu: run if everything is in place.
  const runRef = useRef<() => void>(() => {});
  useEffect(() => {
    const h = () => runRef.current();
    window.addEventListener("stillroom:run", h);
    return () => window.removeEventListener("stillroom:run", h);
  }, []);

  const missing = Object.entries(recipe.inputs)
    .filter(([k, spec]) => spec.required !== false && (inputs[k] == null || inputs[k] === "" || (Array.isArray(inputs[k]) && (inputs[k] as unknown[]).length === 0)))
    .map(([k]) => k);
  const slotCandidates = (requires: string[]) =>
    models.filter((m) => requires.every((c) => m.capabilities.includes(c as never)));
  const unfillable = Object.entries(recipe.modelSlots).filter(([, s]) => slotCandidates(s.requires).length === 0);
  const blocked = running || missing.length > 0 || unfillable.length > 0 || permissionNeeded;
  runRef.current = () => { if (!blocked) onRun(recipe, inputs, pins, reuse); };

  function tick(key: string, on: boolean) {
    const next = new Set(ticked);
    if (on) next.add(key); else next.delete(key);
    setTicked(next);
    setConsentError(null);
    // The last tick is the answer: write every needed line as allowed, then the list is read again.
    if (on && allTicked(consent.needed, next)) {
      setConsentBusy(true);
      api.grantPermissions(recipe.id, consent.needed)
        .then(() => onConsentChanged?.())
        .catch((e) => { setConsentError(String(e)); setTicked(new Set()); })
        .finally(() => setConsentBusy(false));
    }
  }
  function revoke() {
    setConsentBusy(true);
    setConsentError(null);
    setTicked(new Set());
    api.revokePermissions(recipe.id)
      .then(() => onConsentChanged?.())
      .catch((e) => setConsentError(String(e)))
      .finally(() => setConsentBusy(false));
  }

  return (
    <section className="panel">
      <div className="row space">
        <h2 style={{ margin: 0 }}>
          <span className="muted" style={{ marginRight: 10 }}>Recipe:</span>
          <select value={recipe.id} onChange={(e) => onSelect(e.target.value)} className="title-select" aria-label="Recipe">
            {recipes.map((r) => (
              <option key={r.id} value={r.id}>{r.name}</option>
            ))}
          </select>
        </h2>
        <span className="muted mono">{recipe.source} · v{recipe.version ?? "0"}</span>
      </div>
      {recipe.description && <p className="muted">{recipe.description}</p>}

      <ConsentZone consent={consent} ticked={ticked} onTick={tick} onRevoke={revoke} busy={consentBusy} />
      {consentError && <p className="error">Could not write your answer: {consentError}</p>}

      {recipe.stages.filter((st) => (st.inputs ?? []).some((n) => n in recipe.inputs) || st.keep).map((st, i) => (
        <section key={st.id} id={`stage-${st.id}`} className="stage-section">
          {recipe.stages.length > 1 && (() => {
            const est = estimate?.stages.find((e) => e.id === st.id);
            return (
              <h3 className="stage-head"><span className="stage-n">{recipe.stages.indexOf(st) + 1}</span> {st.name}
                {est && <span className="tag">{estimateWord(est)}</span>}
                {st.why && <span className="muted"> · {st.why}</span>}
              </h3>
            );
          })()}
          {st.keep && (() => {
            const k = keeps?.[st.id];
            const w = k ? (inputsKnownUpTo(recipe, st, inputs) ? keepWord(k) : keepHeldWord(k)) : { status: "…", tone: "dim" as const };
            const last = k?.last;
            const changed = last ? last.added + last.replaced + last.removed : 0;
            return (
              <div className="row keep-line">
                <span className={`tag ${w.tone === "amber" ? "amber" : ""}`}>{w.status}</span>
                <span className="muted">
                  This stage remembers between runs (a run keep): what it builds is kept on this Mac in Stillroom's data folder, and deleting the recipe deletes it.
                  {last && (changed ? ` Last run: +${last.added} added, ${last.replaced} replaced, ${last.removed} removed (${last.reason}).` : ` Last run: ${last.reason}.`)}
                </span>
                {onRebuild && k && k.fresh != null && (confirmRebuild === st.id
                  ? <><button className="danger" onClick={() => { setConfirmRebuild(null); onRebuild(st.id); }}>Really rebuild</button><button onClick={() => setConfirmRebuild(null)}>Keep it</button></>
                  : <button disabled={running} onClick={() => setConfirmRebuild(st.id)}>Rebuild…</button>)}
              </div>
            );
          })()}
          {(st.inputs ?? []).filter((n) => n in recipe.inputs).map((name) => (
            <InputField key={name} name={name} spec={recipe.inputs[name]} value={inputs[name]}
              onChange={(v) => setInputs((i) => ({ ...i, [name]: v }))} />
          ))}
          {i === 0 && null}
        </section>
      ))}

      <h3>Models for this run <span className="muted">(model slots)</span></h3>
      {unfillable.some(([, s]) => s.requires.includes("embedding")) && (
        <p className="hint-line"><Glyph name="warning" /> This recipe needs an embedding model (one that turns text into numbers for search). None is installed. In Settings › Models, pull <span className="mono">nomic-embed-text</span> or <span className="mono">bge-m3</span>, then come back.</p>
      )}
      {Object.entries(recipe.modelSlots).map(([slot, spec]) => {
        const candidates = slotCandidates(spec.requires);
        const picked = preview?.resolutions[slot];
        const problem = preview?.problems[slot];
        const pickedName = picked ? (models.find((m) => m.id === picked.modelId)?.name ?? picked.modelId) : null;
        const via = picked ? VIA_WORD[picked.via] ?? picked.via : "";
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
              <option value="">{pickedName && !pins[slot] ? `Auto · ${pickedName}` : "Auto"}</option>
              {candidates.map((m) => (
                <option key={m.id} value={m.id}>{m.name}{m.parameterSize ? ` · ${m.parameterSize}` : ""}</option>
              ))}
            </select>
            <p className="muted slot-pick">
              {problem ? <span className="amber"><Glyph name="warning" /> {problem}</span>
                : picked ? <><Glyph name="ai" /> Will use <span className="mono">{pickedName}</span> <span className={isLocalModel(picked.modelId) ? "tag" : "tag amber"}>{whereWord(picked.modelId)}</span> · {via}</>
                : preview ? "no model yet" : "…"}
            </p>
          </div>
        );
      })}

      <p className="muted">Permissions: {describePermissions(recipe.permissions, recipe.inputs)}. Nothing else is touched.</p>
      <p className="muted">Steps: {recipe.steps.map(stepLabel).join(" → ")}</p>
      {estimate ? (() => {
        // A kept stage that is fresh for these inputs will be skipped: take it out of the total.
        const skipped = estimate.stages.filter((e) => e.once && keeps?.[e.id]?.fresh && inputsKnownUpTo(recipe, recipe.stages.find((st) => st.id === e.id)!, inputs));
        const skipMs = skipped.reduce((n, e) => n + e.avgDurationMs, 0);
        const names = skipped.map((e) => recipe.stages.find((st) => st.id === e.id)?.name ?? e.id);
        return (
        <div className="muted estimate">
          <span>Expect about {aboutMs(Math.max(0, estimate.avgDurationMs - skipMs))} and {fmtTokens(estimate.avgPromptTokens + estimate.avgCompletionTokens)} tokens, from the last {estimate.basedOnRuns} run{estimate.basedOnRuns === 1 ? "" : "s"}{names.length ? ` (${names.join(" and ")} ${names.length === 1 ? "is" : "are"} kept and will not run)` : ""}.</span>
          {estimate.stages.length > 0 && (
            <span className="estimate-stages">
              {estimate.stages.map((e) => {
                const i = recipe.stages.findIndex((st) => st.id === e.id);
                const st = recipe.stages[i];
                return <span key={e.id} className="fact"><span className="stage-n">{i + 1}</span> {st?.name ?? e.id} <span className="muted">· {estimateWord(e)}</span></span>;
              })}
            </span>
          )}
        </div>
        );
      })() : (
        <p className="muted">No previous runs to estimate from. After the first run, Set up says what each stage took and which stages will not need to run again.</p>
      )}

      <label className="row" style={{ margin: "6px 0" }}>
        <input type="checkbox" checked={reuse} onChange={(e) => setReuse(e.target.checked)} />
        <span>Reuse unchanged steps <span className="muted">(selective rerun: a step runs again only if its inputs, prompt, or model changed)</span></span>
      </label>
      <button className="primary" disabled={blocked}
        onClick={() => onRun(recipe, inputs, pins, reuse)}>
        {running ? "Running…" : permissionNeeded ? "Run · needs your permission" : missing.length > 0 ? `Run · needs ${missing.map((k) => recipe.inputs[k]?.label ?? k).join(", ")}` : unfillable.length > 0 ? `Run · no installed model can do ${unfillable.map(([, s]) => s.requires.join(" and ")).join("; ")}` : "Run"}
      </button>
      {folderInputs.length > 0 && (
        <>
          {" "}
          <button disabled={blocked || !watchInput}
            onClick={() => onWatch(recipe, watchInput, inputs, pins)}>
            Watch {folderInputs.length === 1 ? (recipe.inputs[watchInput]?.label ?? watchInput) : "folder"}…
          </button>
          {folderInputs.length > 1 && (
            <select value={watchInput} onChange={(e) => setWatchInput(e.target.value)} style={{ marginLeft: 6 }}>
              {folderInputs.map((k) => <option key={k} value={k}>{recipe.inputs[k].label ?? k}</option>)}
            </select>
          )}
        </>
      )}
      {unfillable.length > 0 && (
        <p className="muted">No installed model has what {unfillable.map(([s]) => s).join(", ")} needs. Pull one in Ollama, then refresh Models in the Library.</p>
      )}
      {folderInputs.length > 0 && missing.length === 0 && (
        <p className="muted">
          Watch <span className="mono">{recipe.inputs[watchInput]?.label ?? watchInput}</span> (a folder watch): the recipe runs with these
          inputs whenever something lands in that folder, while Stillroom is open. Each run continues the last, so files handled before are not handled again.
        </p>
      )}
    </section>
  );
}
