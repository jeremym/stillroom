import { fmtMs } from "../api";
import type { Run, StepRecord } from "../types";

function statusCell(s: StepRecord) {
  if (s.edited) return "edited by you";
  if (s.reusedFrom) return <span title={`from run ${s.reusedFrom}`}>reused</span>;
  if (s.status === "skipped") return <span title={s.skipReason ?? ""}>skipped · {s.skipReason}</span>;
  return s.status;
}

export function TracePanel({ run, steps: unsorted }: { run: Run | null; steps: StepRecord[] }) {
  // Records are stored as they finish (a map after its children); show them as they started.
  const steps = [...unsorted].sort((a, b) => a.position - b.position);
  if (steps.length === 0 && !run) return null;
  const failed = steps.find((s) => s.error);
  const reused = run?.steps.filter((s) => s.reusedFrom).length ?? 0;
  const edited = run?.steps.filter((s) => s.edited) ?? [];
  const regenerated = run?.steps.filter((s) => !s.edited && !s.reusedFrom && s.status === "succeeded").length ?? 0;
  const setAside = run ? Object.keys(run.edits ?? {}).filter((id) => !run.steps.find((s) => s.stepId === id)?.edited) : [];
  const toolCalls = run?.steps.reduce((n, s) => n + (s.toolCalls?.length ?? 0), 0) ?? 0;
  return (
    <section className="panel">
      <h2>How this was made <span className="muted">(execution trace)</span></h2>
      {run && (
        <p className="muted">
          Run {run.id.slice(-8)} · {run.status === "needs_review" ? "paused for review" : run.status}
          {Object.values(run.resolutions ?? {}).map((r) => (
            <span key={r.slot}> · {r.slot} → <span className="mono">{r.modelId}</span> ({r.via})</span>
          ))}
        </p>
      )}
      {run && (reused > 0 || edited.length > 0) && (
        <p className="muted">
          {edited.length > 0 && <>You {run.steps.some((s) => s.edited && s.kind === "review") ? "decided" : "edited"} {edited.map((s) => s.name ?? s.stepId).join(", ")}. {regenerated} steps ran. </>}
          {reused} of {run.steps.length} steps were reused from earlier runs because nothing they depend on changed.
        </p>
      )}
      {toolCalls > 0 && <p className="muted">Agent steps made {toolCalls} tool call{toolCalls === 1 ? "" : "s"}. Open a row to see them.</p>}
      {setAside.map((id) => (
        <p className="muted" key={id}>
          Your earlier edit to <strong>{run?.steps.find((s) => s.stepId === id)?.name ?? id}</strong> was set aside because what feeds it changed, so it was generated fresh.
        </p>
      ))}
      <table>
        <thead>
          <tr><th>#</th><th>Step</th><th>Kind</th><th>Status</th><th>Model</th><th>Tokens in → out</th><th>Time</th></tr>
        </thead>
        <tbody>
          {steps.map((s) => (
            <tr key={s.stepId} className={`${s.status} ${s.parent ? "nested" : ""}`}>
              <td>{s.position + 1}</td>
              <td>
                {s.parent && <span className="muted mono">{s.stepId.replace(/\.[^.]+$/, "")} · </span>}
                {s.name ?? s.stepId}
                {s.toolCalls && s.toolCalls.length > 0 && (
                  <details className="tool-calls">
                    <summary className="muted">{s.toolCalls.length} tool call{s.toolCalls.length === 1 ? "" : "s"}</summary>
                    <ol>
                      {s.toolCalls.map((c, i) => (
                        <li key={i}>
                          <span className="mono">{c.tool}</span> <span className="muted">({fmtMs(c.durationMs)})</span>
                          <pre>{JSON.stringify(c.inputs)}</pre>
                          {c.error ? <pre className="error">{c.error}</pre> : <pre className="muted">{JSON.stringify(c.output)?.slice(0, 300)}</pre>}
                        </li>
                      ))}
                    </ol>
                  </details>
                )}
              </td>
              <td className="muted">{s.kind}</td>
              <td>{statusCell(s)}</td>
              <td className="mono">{s.model ?? ""}</td>
              <td>{s.promptTokens != null ? `${s.promptTokens} → ${s.completionTokens ?? 0}` : ""}</td>
              <td>{fmtMs(s.durationMs)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {failed && <pre className="error">{failed.error}</pre>}
      {run?.error && !failed && <pre className="error">{run.error}</pre>}
    </section>
  );
}
