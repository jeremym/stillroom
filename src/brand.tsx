import { Glyph as KitGlyph, isGlyphName as isKitGlyph, Logo as KitLogo, type GlyphName as KitGlyphName } from "@overtone-foundry/app-kit";

/* Stillroom's own marks and recipe-specific glyph helpers. The kit (@overtone-foundry/app-kit) owns
   the Lucide-backed glyph set; this file adds the alembic, the llama, and what a recipe needs. */

export const ALEMBIC = "M9 3h6M10.5 3v5.5L5 19a1.5 1.5 0 0 0 1.3 2h11.4a1.5 1.5 0 0 0 1.3-2L13.5 8.5V3M8 15h8";
export const LLAMA = "M8 21v-6a4 4 0 0 1 4-4h4V5l2-2v8a6 6 0 0 1-2 4v6M14 21v-4";
const BRAND = { alembic: ALEMBIC, llama: LLAMA } as const;

/** Stillroom's glyph names: the kit's, plus its two brand marks. */
export type GlyphName = KitGlyphName | keyof typeof BRAND;

export function Glyph({ name, className, title }: { name: GlyphName; className?: string; title?: string }) {
  return name in BRAND
    ? <KitGlyph brand={BRAND[name as keyof typeof BRAND]} className={className} title={title} />
    : <KitGlyph name={name as KitGlyphName} className={className} title={title} />;
}

export function isGlyphName(v: unknown): v is GlyphName {
  return isKitGlyph(v) || (typeof v === "string" && v in BRAND);
}

export function Logo({ size = 36, word = true }: { size?: number; word?: boolean }) {
  return <KitLogo mark={ALEMBIC} name="Stillroom" size={size} word={word} />;
}

export const RECIPE_GLYPHS: GlyphName[] = [
  "recipe", "notes", "slides", "audio", "table", "diagram", "mail", "inbox", "links", "globe", "folder", "file", "files",
  "learn", "checklist", "chat", "news", "calendar", "image", "music", "code", "shield", "search", "sparkles", "ai", "tool", "watch",
];

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
