import { fmtMs, fmtTokens } from "../api";
import type { ModelUsage } from "../types";

export function UsagePanel({ usage }: { usage: ModelUsage[] }) {
  if (usage.length === 0) return null;
  return (
    <section className="panel">
      <h2>Usage <span className="muted">(budgets, all runs)</span></h2>
      <table>
        <thead>
          <tr><th>Model</th><th>Calls</th><th>Tokens in</th><th>Tokens out</th><th>Time</th></tr>
        </thead>
        <tbody>
          {usage.map((u) => (
            <tr key={u.modelId}>
              <td className="mono">{u.modelId}</td>
              <td>{u.calls}</td>
              <td>{fmtTokens(u.promptTokens)}</td>
              <td>{fmtTokens(u.completionTokens)}</td>
              <td>{fmtMs(u.totalMs)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="muted">Local models cost time, not dollars. Cost estimates arrive with cloud providers.</p>
    </section>
  );
}
