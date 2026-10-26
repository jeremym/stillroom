import type { ModelsResponse } from "../types";

export function ModelsPanel({ models, onRefresh }: { models: ModelsResponse | null; onRefresh: () => void }) {
  return (
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
          <button onClick={onRefresh}>Refresh</button>
        </>
      ) : (
        <p className="muted">Looking for local model providers…</p>
      )}
    </section>
  );
}
