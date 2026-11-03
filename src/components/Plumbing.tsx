import { Glyph } from "../brand";
import { Pill } from "@overtone-foundry/app-kit";
import { shortModel } from "../models";
import { canPlan, costLine, fmtPlanTime } from "../planWith";
import { suggestPlanner } from "../settings/pull";
import { PullModel } from "./PullModel";
import type { ModelsResponse, PlannerCandidate, PlannerPreview } from "../types";

/* Plumbing: Ollama running, a model that can plan. Needed, but not what the person came for, so on the
   first-run screen it is one status line at the bottom, which unfolds into help only when something is
   missing. The help blocks are the ones the welcome wizard used to show as its first step. */

export type PlumbingState = "loading" | "down" | "empty" | "no-planner" | "ready";

/** What the plumbing is doing: Ollama down, running with no model, running with no model that can plan, or ready. */
export function plumbingState(models: ModelsResponse | null): PlumbingState {
  if (!models) return "loading";
  const provider = models.providers[0];
  if (!provider?.available) return "down";
  const chat = models.models.filter((m) => m.capabilities.includes("text"));
  if (chat.length === 0) return "empty";
  return models.models.some(canPlan) ? "ready" : "no-planner";
}

/** Ollama is not answering: where it should be, how to get it, and a button to look again. */
export function OllamaDown({ baseUrl, onRefreshModels, onOpenSettings }: { baseUrl?: string | null; onRefreshModels: () => void; onOpenSettings: () => void }) {
  return (
    <div className="plumbing-help" data-testid="ollama-down">
      <p>Stillroom asks <strong>Ollama</strong>, a free program that runs AI models on your own computer. Nothing you give a recipe leaves this Mac unless you later turn on a cloud model in Settings › Models, and steps that use one are marked amber.</p>
      <p className="amber"><Glyph name="warning" /> Ollama is not answering at <span className="mono">{baseUrl ?? "http://localhost:11434"}</span>.</p>
      <p className="muted">Install it from <span className="mono">ollama.com</span> and open it, then check again. If Ollama runs on another machine, set its address (and an access key if it needs one) in Settings.</p>
      <div className="row">
        <button className="primary" style={{ marginTop: 0 }} onClick={onRefreshModels}>I've started it, check again</button>
        <button onClick={onOpenSettings}><Glyph name="settings" /> Settings › Models</button>
      </div>
    </div>
  );
}

/** Ollama runs but nothing installed can draft a recipe (plan): the starter to pull, sized to this Mac, pulled in place. */
export function NeedsPlanner({ state, installed, memoryBytes, onRefreshModels }: { state: "empty" | "no-planner"; installed: string[]; memoryBytes: number | null | undefined; onRefreshModels: () => void }) {
  const starter = suggestPlanner(memoryBytes);
  return (
    <div className="plumbing-help" data-testid="needs-planner">
      {state === "empty" ? (
        <p>Ollama is running but has no model yet. Pull <span className="mono">{starter}</span>: it answers in seconds on most Macs, can draft recipes from a goal (plan), and runs on this Mac.</p>
      ) : (
        <p className="amber planner-starter"><Glyph name="warning" /> None of the installed models can draft a recipe from a goal (plan): that needs a model that answers in a fixed shape (JSON). Pull <span className="mono">{starter}</span>: it drafts recipes and runs on this Mac.</p>
      )}
      <PullModel enabled installed={installed} suggestion={starter} prompt onDone={onRefreshModels} />
    </div>
  );
}

/** The planner's part of the line: "qwen3:14b will plan (fits in memory, plans in 48 s)". */
export function plannerWords(chosen: PlannerCandidate | null, memoryBytes: number | null | undefined): string | null {
  if (!chosen) return null;
  const cost = costLine(chosen, memoryBytes);
  const fit = chosen.local && !chosen.fits ? "larger than this Mac holds comfortably" : cost.fit;
  const timing = chosen.measuredMs != null && chosen.measuredClean ? `, plans in ${fmtPlanTime(chosen.measuredMs)}` : "";
  return `${shortModel(chosen.modelId)} will plan (${fit}${timing})`;
}

interface LineProps {
  models: ModelsResponse | null;
  preview: PlannerPreview | null;
  /** The model the draft would use: the person's pick, else Auto's. */
  chosen: PlannerCandidate | null;
  onRefreshModels: () => void;
  onOpenSettings: () => void;
  /** Change the planner: shows the Plan with picker. */
  onChange: () => void;
  changing: boolean;
}

/** One line: provider, model count, which model will plan and at what cost, and a way to change it.
    When Ollama is down or no model can plan, the help for that case unfolds beneath. */
export function PlumbingLine({ models, preview, chosen, onRefreshModels, onOpenSettings, onChange, changing }: LineProps) {
  const state = plumbingState(models);
  const provider = models?.providers[0];
  const count = models?.models.length ?? 0;
  const words = plannerWords(chosen, preview?.memoryBytes);
  return (
    <div className="plumbing" data-testid="plumbing" data-state={state}>
      <p className="plumbing-line">
        <Pill ok={state === "loading" ? undefined : state !== "down"}>
          {provider?.label ?? "Ollama"} · {state === "loading" ? "checking…" : state === "down" ? "not running" : `${count} model${count === 1 ? "" : "s"}`}
        </Pill>
        {state === "ready" && words && <><span className="sep">·</span><span>{words}</span><span className="sep">·</span><button className="link" aria-expanded={changing} onClick={onChange}>{changing ? "done" : "change"}</button></>}
        {state === "ready" && !words && <><span className="sep">·</span><span className="muted">choosing a planner…</span></>}
        {state === "empty" && <><span className="sep">·</span><span className="amber">no model yet</span></>}
        {state === "no-planner" && <><span className="sep">·</span><span className="amber">none can plan</span></>}
      </p>
      {state === "down" && <OllamaDown baseUrl={provider?.baseUrl} onRefreshModels={onRefreshModels} onOpenSettings={onOpenSettings} />}
      {(state === "empty" || state === "no-planner") && (
        <NeedsPlanner state={state} installed={models?.models.map((m) => m.name) ?? []} memoryBytes={preview?.memoryBytes} onRefreshModels={onRefreshModels} />
      )}
    </div>
  );
}
