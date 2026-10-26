import { useEffect, useId, useState } from "react";
import mermaid from "mermaid";

let initialized = false;
function ensureInit() {
  if (initialized) return;
  const dark = window.matchMedia?.("(prefers-color-scheme: dark)").matches;
  mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: dark ? "dark" : "neutral", fontFamily: "inherit" });
  initialized = true;
}

export interface DiagramValue {
  format?: string;
  kind?: string;
  title?: string;
  source: string;
}

export function asDiagram(value: unknown): DiagramValue | null {
  if (typeof value === "string") return { source: value };
  if (typeof value === "object" && value !== null && typeof (value as { source?: unknown }).source === "string") {
    return value as DiagramValue;
  }
  return null;
}

export function DiagramView({ diagram, onSvg }: { diagram: DiagramValue; onSvg?: (svg: string | null) => void }) {
  const id = useId().replace(/[^a-zA-Z0-9]/g, "");
  const [svg, setSvg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showSource, setShowSource] = useState(false);

  useEffect(() => {
    let cancelled = false;
    ensureInit();
    setSvg(null);
    setError(null);
    mermaid
      .render(`m${id}`, diagram.source)
      .then((r) => {
        if (cancelled) return;
        setSvg(r.svg);
        onSvg?.(r.svg);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e?.message ?? e));
        onSvg?.(null);
      });
    return () => {
      cancelled = true;
    };
  }, [diagram.source, id]);

  return (
    <div className="diagram">
      {svg && <div className="diagram-svg" dangerouslySetInnerHTML={{ __html: svg }} />}
      {error && (
        <pre className="error">This diagram did not render: {error}</pre>
      )}
      <button className="link" onClick={() => setShowSource((s) => !s)}>
        {showSource ? "Hide source" : "Show source"} <span className="muted">(Mermaid)</span>
      </button>
      {showSource && <pre>{diagram.source}</pre>}
    </div>
  );
}
