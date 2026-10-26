import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { ModelsResponse, Recipe, Run, RunSummary, RuntimeStatus, StepRecord } from "./types";
import "./App.css";

const PIN_KEY = "stillroom.pins.summarize-file";

function loadPins(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(PIN_KEY) ?? "{}");
  } catch {
    return {};
  }
}

function fmtMs(ms: number | null) {
  if (ms == null) return "";
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

function App() {
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [models, setModels] = useState<ModelsResponse | null>(null);
  const [recipe, setRecipe] = useState<Recipe | null>(null);
  const [file, setFile] = useState<string | null>(null);
  const [pins, setPins] = useState<Record<string, string>>(loadPins);
  const [liveSteps, setLiveSteps] = useState<StepRecord[]>([]);
  const [run, setRun] = useState<Run | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [history, setHistory] = useState<RunSummary[]>([]);

  const refreshModels = useCallback(() => {
    invoke<ModelsResponse>("list_models").then(setModels).catch((e) => setError(String(e)));
  }, []);
  const refreshHistory = useCallback(() => {
    invoke<RunSummary[]>("list_runs", { limit: 20 }).then(setHistory).catch(() => {});
  }, []);

  useEffect(() => {
    invoke<RuntimeStatus>("runtime_status").then(setStatus).catch((e) => setError(String(e)));
    invoke<Recipe>("builtin_recipe").then(setRecipe).catch((e) => setError(String(e)));
    refreshModels();
    refreshHistory();
    const unlisten = listen<StepRecord>("run:step", (ev) => {
      setLiveSteps((prev) => {
        const next = prev.filter((s) => s.stepId !== ev.payload.stepId);
        next.push(ev.payload);
        return next.sort((a, b) => a.position - b.position);
      });
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [refreshModels, refreshHistory]);

  useEffect(() => {
    try {
      localStorage.setItem(PIN_KEY, JSON.stringify(pins));
    } catch {
      /* per-viewer convenience only */
    }
  }, [pins]);

  async function pickFile() {
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Text", extensions: ["txt", "md", "markdown"] }],
    });
    if (typeof picked === "string") setFile(picked);
  }

  async function startRun() {
    if (!file) return;
    setRunning(true);
    setError(null);
    setRun(null);
    setLiveSteps([]);
    try {
      const result = await invoke<Run>("run_builtin", { args: { inputs: { file }, pins } });
      setRun(result);
      refreshHistory();
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  async function openRun(id: string) {
    const r = await invoke<Run | null>("get_run", { id });
    if (r) {
      setRun(r);
      setLiveSteps([]);
    }
  }

  const steps = run?.steps ?? liveSteps;
  const textModels = models?.models.filter((m) => m.capabilities.includes("text")) ?? [];
  const summary = run?.outputs?.summary;

  return (
    <main className="shell">
      <header>
        <h1>Stillroom</h1>
        <p className="tagline">Describe the goal. Get the result. Open the door.</p>
      </header>

      <section className="panel">
        <h2>Models <span className="muted">(providers and installed models)</span></h2>
        {models ? (
          <>
            <ul className="providers">
              {models.providers.map((p) => (
                <li key={p.id}>
                  <span className={p.available ? "dot ok" : "dot bad"} />
                  {p.label} <span className="muted">{p.baseUrl}</span>
                  {!p.available && <span className="muted"> · not running</span>}
                </li>
              ))}
            </ul>
            {models.models.length > 0 ? (
              <table>
                <thead>
                  <tr><th>Model</th><th>Size</th><th>Context</th><th>Capabilities</th></tr>
                </thead>
                <tbody>
                  {models.models.map((m) => (
                    <tr key={m.id}>
                      <td>{m.name}</td>
                      <td>{m.parameterSize ?? ""}</td>
                      <td>{m.contextWindow ? `${Math.round(m.contextWindow / 1024)}k` : ""}</td>
                      <td className="muted">{m.capabilities.join(", ")}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <p className="muted">No models found. Start Ollama and pull a model, then refresh.</p>
            )}
            <button onClick={refreshModels}>Refresh</button>
          </>
        ) : (
          <p className="muted">Looking for local model providers…</p>
        )}
      </section>

      <section className="panel">
        <h2>{recipe?.name ?? "Recipe"}</h2>
        {recipe?.description && <p className="muted">{recipe.description}</p>}

        <div className="row">
          <button onClick={pickFile}>Choose file…</button>
          <span className="mono">{file ?? "no file selected"}</span>
        </div>

        {recipe &&
          Object.entries(recipe.modelSlots).map(([slot, spec]) => (
            <div className="row" key={slot}>
              <label>
                <strong>{slot}</strong> <span className="muted">(model slot: needs {spec.requires.join(", ")})</span>
                <select
                  value={pins[slot] ?? ""}
                  onChange={(e) => {
                    const v = e.target.value;
                    setPins((p) => {
                      const next = { ...p };
                      if (v) next[slot] = v;
                      else delete next[slot];
                      return next;
                    });
                  }}
                >
                  <option value="">Auto (prefer {spec.preference ?? "accuracy"})</option>
                  {textModels.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name}{m.parameterSize ? ` · ${m.parameterSize}` : ""}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          ))}

        <button className="primary" disabled={!file || running || textModels.length === 0} onClick={startRun}>
          {running ? "Running…" : "Run"}
        </button>
        {error && <pre className="error">{error}</pre>}
      </section>

      {(steps.length > 0 || run) && (
        <section className="panel">
          <h2>How this was made <span className="muted">(execution trace)</span></h2>
          {run && (
            <p className="muted">
              Run {run.id.slice(-8)} · {run.status}
              {Object.values(run.resolutions ?? {}).map((r) => (
                <span key={r.slot}> · {r.slot} → {r.modelId} ({r.via})</span>
              ))}
            </p>
          )}
          <table>
            <thead>
              <tr><th>#</th><th>Step</th><th>Kind</th><th>Status</th><th>Model</th><th>Tokens</th><th>Time</th></tr>
            </thead>
            <tbody>
              {steps.map((s) => (
                <tr key={s.stepId} className={s.status}>
                  <td>{s.position + 1}</td>
                  <td>{s.name ?? s.stepId}</td>
                  <td className="muted">{s.kind}</td>
                  <td>{s.status}</td>
                  <td className="mono">{s.model ?? ""}</td>
                  <td>{s.promptTokens != null ? `${s.promptTokens} → ${s.completionTokens ?? 0}` : ""}</td>
                  <td>{fmtMs(s.durationMs)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {steps.some((s) => s.error) && (
            <pre className="error">{steps.find((s) => s.error)?.error}</pre>
          )}
        </section>
      )}

      {typeof summary === "string" && (
        <section className="panel">
          <h2>Summary <span className="muted">(artifact)</span></h2>
          <pre className="artifact">{summary}</pre>
        </section>
      )}

      {history.length > 0 && (
        <section className="panel">
          <h2>Previous runs</h2>
          <ul className="history">
            {history.map((h) => (
              <li key={h.id}>
                <button className="link" onClick={() => openRun(h.id)}>
                  {new Date(h.startedAt).toLocaleString()} · {h.recipeName} · {h.status} · {h.promptTokens + h.completionTokens} tokens
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      <footer className="muted mono">
        {status ? `${status.schemaTitle} v0 · tools: ${status.tools.join(", ")} · ${status.runCount} runs · ${status.dbPath}` : "…"}
      </footer>
    </main>
  );
}

export default App;
