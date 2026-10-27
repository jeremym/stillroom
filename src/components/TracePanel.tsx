import { fmtMs } from "../api";
import type { Run, StepRecord } from "../types";

export function TracePanel({ run, steps }: { run: Run | null; steps: StepRecord[] }) {
  if (steps.length === 0 && !run) return null;
  const failed = steps.find((s) => s.error);
  return (
    <section className="panel">
      <h2>How this was made <span className="muted">(execution trace)</span></h2>
      {run && (
        <p className="muted">
          Run {run.id.slice(-8)} · {run.status}
          {Object.values(run.resolutions ?? {}).map((r) => (
            <span key={r.slot}> · {r.slot} → <span className="mono">{r.modelId}</span> ({r.via})</span>
          ))}
        </p>
      )}
      {run && run.steps.some((s) => s.reusedFrom) && (
        <p className="muted">
          {run.steps.filter((s) => s.reusedFrom).length} of {run.steps.length} steps were reused from earlier runs because nothing they depend on changed.
        </p>
      )}
      <table>
        <thead>
          <tr><th>#</th><th>Step</th><th>Kind</th><th>Status</th><th>Model</th><th>Tokens in → out</th><th>Time</th></tr>
        </thead>
        <tbody>
          {steps.map((s) => (
            <tr key={s.stepId} className={s.status}>
              <td>{s.position + 1}</td>
              <td>{s.name ?? s.stepId}</td>
              <td className="muted">{s.kind}</td>
              <td>{s.reusedFrom ? <span title={`from run ${s.reusedFrom}`}>reused</span> : s.status}</td>
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
