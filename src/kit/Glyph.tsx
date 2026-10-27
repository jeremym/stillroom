/* The glyph set: one inline sprite, 2px strokes in currentColor, glow by CSS. Keep every glyph a
   plain outline so it reads at 16px and at 40px. */

export type GlyphName =
  | "alembic" | "file" | "files" | "folder" | "text" | "links" | "data" | "number" | "yesno"
  | "program" | "ai" | "agent" | "foreach" | "approval" | "branch" | "recipe"
  | "in" | "out" | "notes" | "slides" | "diagram" | "audio" | "table" | "json"
  | "local" | "cloud" | "llama" | "watch" | "history" | "models" | "settings" | "warning" | "power" | "door";

const PATHS: Record<GlyphName, string> = {
  alembic: "M9 3h6M10.5 3v5.5L5 19a1.5 1.5 0 0 0 1.3 2h11.4a1.5 1.5 0 0 0 1.3-2L13.5 8.5V3M8 15h8",
  file: "M6 3h8l4 4v14H6zM14 3v4h4M9 12h6M9 16h6",
  files: "M8 3h7l3 3v11H8zM15 3v3h3M5 7v14h10",
  folder: "M3 6h6l2 2h10v12H3z",
  text: "M5 6h14M5 11h14M5 16h9",
  links: "M10 14a4 4 0 0 0 5.6 0l3-3a4 4 0 0 0-5.6-5.6l-1 1M14 10a4 4 0 0 0-5.6 0l-3 3a4 4 0 0 0 5.6 5.6l1-1",
  data: "M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zM4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3",
  number: "M9 4L7 20M17 4l-2 16M4 9h16M3 15h16",
  yesno: "M4 12l5 5L20 6",
  program: "M4 4h13l3 3v13H4zM8 4v5h7V4M7 14h10v6H7z",
  ai: "M12 3a7 7 0 0 0-7 7c0 2.5 1.2 4 2.5 5.4V20h9v-4.6C17.8 14 19 12.5 19 10a7 7 0 0 0-7-7zM8 11l1.5-2 1.5 4 1.5-5 1.5 4 1.5-2",
  agent: "M11 3a7 7 0 0 0-7 7c0 2.5 1.2 4 2.5 5.4V20h7M7 11l1.3-2 1.3 4 1.3-5 1.3 4 1.3-2M15.5 13.5l5 5-1.5 1.5-5-5a3 3 0 0 1-3.5-4l2 2 1.5-1.5-2-2a3 3 0 0 1 4 3.5z",
  foreach: "M3 9h13v12H3zM6 6h13v12M9 3h12v12",
  approval: "M5.5 13V7.5a1.25 1.25 0 0 1 2.5 0V12M8.5 12V4.5a1.25 1.25 0 0 1 2.5 0V12M11.5 12V3.5a1.25 1.25 0 0 1 2.5 0V12M14.5 12V5.5a1.25 1.25 0 0 1 2.5 0V14.5c0 3.6-2.7 6.5-6.2 6.5h-1.4c-2 0-3.8-1-4.8-2.7L3 14.5a1.4 1.4 0 0 1 2.3-1.6L5.5 13",
  branch: "M6 3v12M6 15a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 9c0 4-4 5-8 6",
  recipe: "M5 3h11l3 3v15H5zM9 9h6M9 13h6M9 17h4",
  in: "M3 12h11M10 8l4 4-4 4M17 5h4v14h-4",
  out: "M5 3h10l4 4v9M15 3v4h4M9 9h5M9 13h7M9 17h4M14 20h7M18 17l3 3-3 3",
  notes: "M5 6h14M5 11h14M5 16h9",
  slides: "M3 5h18v12H3zM12 17v3M8 20h8",
  diagram: "M4 4h6v5H4zM14 15h6v5h-6zM7 9v4h10v2",
  audio: "M4 12h2M8 8v8M12 5v14M16 8v8M20 11v2",
  table: "M3 5h18v14H3zM3 10h18M3 15h18M9 5v14M15 5v14",
  json: "M8 4c-2 0-3 1-3 3v2c0 1.5-1 2-2 2 1 0 2 .5 2 2v3c0 2 1 3 3 3M16 4c2 0 3 1 3 3v2c0 1.5 1 2 2 2-1 0-2 .5-2 2v3c0 2-1 3-3 3",
  local: "M3 11l9-7 9 7v9H3zM10 20v-5h4v5",
  cloud: "M7 18a4 4 0 0 1-.5-8 6 6 0 0 1 11.5 1.5A3.5 3.5 0 0 1 17.5 18zM12 15v-4M10 13l2-2 2 2",
  llama: "M8 21v-6a4 4 0 0 1 4-4h4V5l2-2v8a6 6 0 0 1-2 4v6M14 21v-4",
  watch: "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z",
  history: "M12 3a9 9 0 1 0 9 9M12 7v5l3 3M3 4v4h4",
  models: "M7 7h10v10H7zM10 3v4M14 3v4M10 17v4M14 17v4M3 10h4M3 14h4M17 10h4M17 14h4",
  settings: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 3v2M12 19v2M3 12h2M19 12h2",
  warning: "M12 3L2 21h20zM12 10v5M12 18v.5",
  power: "M12 3v8M6.3 7a8 8 0 1 0 11.4 0",
  door: "M5 3h11v18H5zM13 12h.5M16 6l4 2v10l-4 2",
};

export const GLYPH_NAMES = Object.keys(PATHS) as GlyphName[];

export function Glyph({ name, className = "", title }: { name: GlyphName; className?: string; title?: string }) {
  return (
    <svg className={`glyph ${className}`} viewBox="0 0 24 24" aria-hidden={title ? undefined : true} role={title ? "img" : undefined}>
      {title && <title>{title}</title>}
      <path d={PATHS[name]} />
    </svg>
  );
}

/** The glyph for a recipe input of the given type. */
export function inputGlyph(type: string): GlyphName {
  switch (type) {
    case "file": return "file";
    case "files": return "files";
    case "directory": return "folder";
    case "url": case "urls": return "links";
    case "json": return "data";
    case "number": return "number";
    case "boolean": return "yesno";
    default: return "text";
  }
}

/** The glyph for a step kind. */
export function stepGlyph(kind: string): GlyphName {
  switch (kind) {
    case "tool": return "program";
    case "model": return "ai";
    case "agent": return "agent";
    case "map": return "foreach";
    case "review": return "approval";
    case "branch": return "branch";
    case "recipe": return "recipe";
    default: return "program";
  }
}

/** The glyph for a result of the given output type. */
export function outputGlyph(type: string): GlyphName {
  switch (type) {
    case "slides": return "slides";
    case "diagram": return "diagram";
    case "audio": return "audio";
    case "table": return "table";
    case "json": return "json";
    case "markdown": case "text": return "notes";
    default: return "out";
  }
}
