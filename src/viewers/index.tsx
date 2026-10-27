import type { ReactNode } from "react";
import Markdown from "react-markdown";
import { DiagramView, asDiagram } from "../components/DiagramView";
import { SlidesView, isDeck } from "../components/SlidesView";
import { AudioView, ScriptView, isAudio, isScript } from "../components/AudioView";
import { TableView, isRows } from "./TableView";
import { Spine, Cards, isSpine, isCards } from "./learning";

/* The viewer registry. An artifact is shown by its declared output type first, then by the
   shape of its value, then as text or JSON. Recipes never name a viewer; they declare a type. */

export interface ViewerProps {
  kind: string;
  value: unknown;
  onSvg?: (svg: string | null) => void;
}

export function pickViewer({ kind, value, onSvg }: ViewerProps): ReactNode {
  if (kind === "audio" && isAudio(value)) return <AudioView audio={value} />;
  if (kind === "slides" && isDeck(value)) return <SlidesView deck={value} />;
  if (kind === "diagram") {
    const d = asDiagram(value);
    if (d) return (
      <>
        {d.title && <p><strong>{d.title}</strong></p>}
        <DiagramView diagram={d} onSvg={onSvg} />
      </>
    );
  }
  if (kind === "table" && isRows(value)) return <TableView rows={value} />;
  if (kind === "markdown" && typeof value === "string") return <div className="markdown"><Markdown>{value}</Markdown></div>;
  if (isScript(value)) return <ScriptView script={value} />;
  if (isSpine(value)) return <Spine spine={value} />;
  if (isCards(value)) return <Cards cards={value.cards} />;
  if (isRows(value)) return <TableView rows={value} />;
  if (typeof value === "string") return <pre className="artifact">{value}</pre>;
  if (value == null) return <p className="muted">Nothing here yet.</p>;
  return <pre>{JSON.stringify(value, null, 2)}</pre>;
}
