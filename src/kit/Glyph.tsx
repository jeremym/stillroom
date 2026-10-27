import type { ComponentType, SVGProps } from "react";
import {
  AlignLeft, ArrowLeft, AudioLines, BookOpen, Braces, Brain, BrainCog, Calendar, Check, ChevronDown, CircleHelp, CloudUpload, Code, Copy, Cpu,
  Database, DoorOpen, Eye, File, FileOutput, Files, FlaskConical, Folder, GitBranch, Globe, GraduationCap, Hand, Hash, History, House, Image,
  Inbox, Layers, Link, ListChecks, LogIn, Mail, MessageSquare, Music, Newspaper, Pencil, Play, Plus, Power, Presentation, Save, Search, Settings,
  Shield, Sparkles, Square, Table, Timer, Trash2, TriangleAlert, Workflow, Wrench, X,
} from "lucide-react";

/* The glyph set: semantic names mapped to Lucide icons (ISC licence), drawn as 1.8 px strokes in
   currentColor so the phosphor colour and the glow come from CSS. Two brand marks (the alembic and
   the llama provider mark) are our own paths. Apps speak in meaning ("approval"), never icon names. */

export type GlyphName =
  | "alembic" | "llama"
  | "file" | "files" | "folder" | "text" | "links" | "data" | "number" | "yesno"
  | "program" | "ai" | "agent" | "foreach" | "approval" | "branch" | "recipe"
  | "in" | "out" | "notes" | "slides" | "diagram" | "audio" | "table" | "json"
  | "local" | "cloud" | "watch" | "history" | "models" | "settings" | "warning" | "power" | "door"
  | "plus" | "close" | "search" | "edit" | "copy" | "delete" | "play" | "stop" | "down" | "help" | "sparkles" | "back" | "timer" | "tool"
  | "mail" | "globe" | "calendar" | "image" | "music" | "code" | "shield" | "inbox" | "news" | "learn" | "checklist" | "chat";

type Icon = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string; strokeWidth?: number | string }>;

const LUCIDE: Record<Exclude<GlyphName, "alembic" | "llama">, Icon> = {
  file: File, files: Files, folder: Folder, text: AlignLeft, links: Link, data: Database, number: Hash, yesno: Check,
  program: Save, ai: Brain, agent: BrainCog, foreach: Layers, approval: Hand, branch: GitBranch, recipe: BookOpen,
  in: LogIn, out: FileOutput, notes: AlignLeft, slides: Presentation, diagram: Workflow, audio: AudioLines, table: Table, json: Braces,
  local: House, cloud: CloudUpload, watch: Eye, history: History, models: Cpu, settings: Settings, warning: TriangleAlert, power: Power, door: DoorOpen,
  plus: Plus, close: X, search: Search, edit: Pencil, copy: Copy, delete: Trash2, play: Play, stop: Square, down: ChevronDown, help: CircleHelp,
  sparkles: Sparkles, back: ArrowLeft, timer: Timer, tool: Wrench,
  mail: Mail, globe: Globe, calendar: Calendar, image: Image, music: Music, code: Code, shield: Shield, inbox: Inbox, news: Newspaper,
  learn: GraduationCap, checklist: ListChecks, chat: MessageSquare,
};

const BRAND: Record<"alembic" | "llama", string> = {
  alembic: "M9 3h6M10.5 3v5.5L5 19a1.5 1.5 0 0 0 1.3 2h11.4a1.5 1.5 0 0 0 1.3-2L13.5 8.5V3M8 15h8",
  llama: "M8 21v-6a4 4 0 0 1 4-4h4V5l2-2v8a6 6 0 0 1-2 4v6M14 21v-4",
};

export const GLYPH_NAMES = [...(Object.keys(BRAND) as GlyphName[]), ...(Object.keys(LUCIDE) as GlyphName[])];

/** Glyphs a user may pick for a recipe: the ones that describe what a recipe does. */
export const RECIPE_GLYPHS: GlyphName[] = [
  "recipe", "notes", "slides", "audio", "table", "diagram", "mail", "inbox", "links", "globe", "folder", "file", "files",
  "learn", "checklist", "chat", "news", "calendar", "image", "music", "code", "shield", "search", "sparkles", "ai", "tool", "watch",
];

export function Glyph({ name, className = "", title }: { name: GlyphName; className?: string; title?: string }) {
  if (name === "alembic" || name === "llama") {
    return (
      <svg className={`glyph ${className}`} viewBox="0 0 24 24" aria-hidden={title ? undefined : true} role={title ? "img" : undefined}>
        {title && <title>{title}</title>}
        <path d={BRAND[name]} />
      </svg>
    );
  }
  const Icon = LUCIDE[name] ?? FlaskConical;
  return <Icon className={`glyph ${className}`} strokeWidth={1.8} aria-hidden={title ? undefined : true} role={title ? "img" : undefined}>{title && <title>{title}</title>}</Icon>;
}

export function isGlyphName(v: unknown): v is GlyphName {
  return typeof v === "string" && (v in LUCIDE || v in BRAND);
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
