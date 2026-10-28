import { useState } from "react";
import { Glyph, Logo, isGlyphName } from "../brand";
import { Pill, Zone } from "@overtone-foundry/app-kit";
import { PullModel } from "./PullModel";
import type { ModelsResponse, RecipeInfo, Settings } from "../types";

/* Power on: the first launch. Three short screens that end in Set up with a recipe chosen.
   Everything here is the same controls the app uses elsewhere, with a sentence of help beside each. */

interface Props {
  models: ModelsResponse | null;
  settings: Settings;
  recipes: RecipeInfo[];
  onSave: (patch: Record<string, unknown>) => void;
  onOpenSettings: () => void;
  onRefreshModels: () => void;
  onPickRecipe: (id: string) => void;
  onSkip: () => void;
}

export function Welcome({ models, settings, recipes, onSave, onOpenSettings, onRefreshModels, onPickRecipe, onSkip }: Props) {
  const [step, setStep] = useState(0);
  const provider = models?.providers[0];
  const installed = models?.models ?? [];
  const chat = installed.filter((m) => m.capabilities.includes("text" as never));
  const ready = Boolean(provider?.available) && chat.length > 0;
  const preference = settings.models?.preference ?? "accuracy";
  const textDefault = settings.models?.byCapability?.text ?? "";

  if (step === 0) {
    return (
      <div className="boot">
        <Logo size={64} />
        <p className="boot-line glow">Describe the goal. Get the result. See how it was made.</p>
        <p className="muted boot-sub">Stillroom runs recipes: reusable processes that turn your files into notes, slides, tables, briefs, and more, using models that run on this Mac. Three short screens and you will have made something.</p>
        <div className="row">
          <button className="primary" style={{ marginTop: 0 }} onClick={() => setStep(1)}><Glyph name="power" /> Begin</button>
          <button className="quiet" onClick={onSkip}>Skip the welcome</button>
        </div>
      </div>
    );
  }

  if (step === 1) {
    return (
      <>
        <Zone title="Where will the thinking happen?" sub="step 1 of 3" className="accent"
          aside={provider && <Pill ok={provider.available}>{provider.label} · {provider.available ? `${installed.length} model${installed.length === 1 ? "" : "s"}` : "not running"}</Pill>}>
          <p>Stillroom asks <strong>Ollama</strong>, a free program that runs AI models on your own computer. Nothing you give a recipe leaves this Mac unless you later turn on Codex in Settings › Models, and steps that use it are marked amber.</p>
          {!provider?.available ? (
            <>
              <p className="amber"><Glyph name="warning" /> Ollama is not answering at <span className="mono">{provider?.baseUrl ?? "http://localhost:11434"}</span>.</p>
              <p className="muted">Install it from <span className="mono">ollama.com</span> and open it, then check again. If Ollama runs on another machine, set its address (and an access key if it needs one) in Settings.</p>
              <div className="row">
                <button className="primary" style={{ marginTop: 0 }} onClick={onRefreshModels}>I've started it, check again</button>
                <button onClick={onOpenSettings}><Glyph name="settings" /> Settings › Models</button>
              </div>
            </>
          ) : chat.length === 0 ? (
            <>
              <p>Ollama is running but has no model yet. Pull one; a model around 8 billion parameters answers in seconds on most Macs.</p>
              <PullModel enabled installed={installed.map((m) => m.name)} onDone={onRefreshModels} />
            </>
          ) : (
            <>
              <p><Glyph name="yesno" /> Ready: {chat.length === 1 ? "one model" : `${chat.length} models`} can read and write ({chat.map((m) => m.name).slice(0, 4).join(", ")}{chat.length > 4 ? ", …" : ""}).</p>
              <details className="trace-table">
                <summary className="muted">Pull another model</summary>
                <PullModel enabled installed={installed.map((m) => m.name)} onDone={onRefreshModels} />
              </details>
            </>
          )}
        </Zone>
        <Zone title="How should it choose?" sub="you can change this any time in Settings › Models">
          <div className="defaults">
            <span className="label">Prefer</span>
            <span>
              <span className="segmented" role="group" aria-label="Preference">
                {(["accuracy", "speed"] as const).map((p) => <button key={p} aria-pressed={preference === p} onClick={() => onSave({ models: { ...settings.models, preference: p } })}>{p}</button>)}
              </span>
              <span className="muted"> · accuracy picks the largest fitting model, speed the smallest</span>
            </span>
            <span className="label">Writing model</span>
            <span>
              <select value={textDefault} onChange={(e) => { const by = { ...(settings.models?.byCapability ?? {}) }; if (e.target.value) by.text = e.target.value; else delete by.text; onSave({ models: { ...settings.models, byCapability: by } }); }} disabled={chat.length === 0}>
                <option value="">Auto · let each recipe decide</option>
                {chat.map((m) => <option key={m.id} value={m.id}>{m.name}{m.parameterSize ? ` · ${m.parameterSize}` : ""}</option>)}
              </select>
              <span className="muted"> · optional: one model for every step that writes</span>
            </span>
          </div>
          <p className="muted">Cloud models are optional, off, and will be marked amber whenever one is used.</p>
          <div className="row">
            <button className="primary" style={{ marginTop: 0 }} disabled={!ready} onClick={() => setStep(2)}>{ready ? "Next: pick a recipe" : "Waiting for a model…"}</button>
            <button className="quiet" onClick={onSkip}>Skip the welcome</button>
          </div>
        </Zone>
      </>
    );
  }

  return (
    <Zone title="What should it make first?" sub="step 2 of 3 · pick a recipe, then choose a file and press Run" className="accent">
      <div className="cards recipe-grid">
        {recipes.map((r) => (
          <button key={r.id} className="card" onClick={() => onPickRecipe(r.id)}>
            <span className="card-title"><Glyph name={isGlyphName(r.icon) ? r.icon : "recipe"} /> {r.name}</span>
            {r.description && <span className="dim">{r.description}</span>}
            <span className="tags"><span className="tag">{r.steps.length} steps</span>{r.id === "summarize-file" && <span className="tag live">fastest first run</span>}</span>
          </button>
        ))}
      </div>
      <p className="muted" style={{ marginTop: 12 }}>Step 3 is the recipe's own Set up: choose what it reads, press Run, and watch the Dial follow the run to Results.</p>
      <div className="row">
        <button onClick={() => setStep(1)}><Glyph name="back" /> Back</button>
        <button className="quiet" onClick={onSkip}>Skip the welcome</button>
      </div>
    </Zone>
  );
}
