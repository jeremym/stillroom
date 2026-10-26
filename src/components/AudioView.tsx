import { convertFileSrc } from "@tauri-apps/api/core";

export interface AudioValue {
  path: string;
  format?: string;
  durationSeconds?: number | null;
  voice?: string;
  chars?: number;
}

export function isAudio(v: unknown): v is AudioValue {
  return typeof v === "object" && v !== null && typeof (v as { path?: unknown }).path === "string";
}

function fmtDuration(s: number | null | undefined) {
  if (s == null) return "";
  const m = Math.floor(s / 60);
  const r = Math.round(s % 60);
  return m > 0 ? `${m} min ${r} s` : `${r} s`;
}

export function AudioView({ audio }: { audio: AudioValue }) {
  return (
    <div className="audio">
      <audio controls preload="metadata" src={convertFileSrc(audio.path)} style={{ width: "100%" }} />
      <p className="muted">
        {fmtDuration(audio.durationSeconds)}
        {audio.voice ? ` · voice: ${audio.voice}` : ""}
        {audio.chars ? ` · ${audio.chars.toLocaleString()} characters` : ""}
        {" · "}<span className="mono">{audio.path}</span>
      </p>
    </div>
  );
}

export interface Script {
  title: string;
  chapters: { title: string; text: string; concept: string }[];
}

export function isScript(v: unknown): v is Script {
  return typeof v === "object" && v !== null && Array.isArray((v as { chapters?: unknown }).chapters);
}

export function ScriptView({ script }: { script: Script }) {
  const words = script.chapters.reduce((n, c) => n + c.text.split(/\s+/).filter(Boolean).length, 0);
  return (
    <div className="script">
      <p className="muted">{script.chapters.length} chapters · about {words.toLocaleString()} words · roughly {Math.round(words / 150)} min spoken</p>
      {script.chapters.map((c, i) => (
        <section key={i} className="chapter">
          <h4>{i + 1}. {c.title}{c.concept && <span className="muted mono"> · {c.concept}</span>}</h4>
          <p>{c.text}</p>
        </section>
      ))}
    </div>
  );
}
