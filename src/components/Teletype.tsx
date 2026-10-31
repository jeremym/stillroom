import { useEffect, useMemo, useRef } from "react";
import { shortModel, whereWord } from "../models";
import { fmtMs, fmtTokens } from "../api";
import type { Stage, StepRecord } from "../types";

/* The teletype: the run as a typed log, one line per event as it happens. The table in TracePanel
   is the same facts for reading afterwards; this is for watching. */

interface Line { at: number; time: string; mark: string; text: string; tone: "" | "dim" | "amber" | "red" | "bright"; indent?: boolean; head?: boolean }

function clock(iso: string | null): { at: number; time: string } {
  if (!iso) return { at: 0, time: "        " };
  const d = new Date(iso);
  return { at: d.getTime(), time: d.toLocaleTimeString([], { hour12: false }) };
}

function kindWord(k: string) {
  return k === "model" ? "AI step" : k === "agent" ? "AI agent" : k === "tool" ? "program" : k === "map" ? "for each" : k === "review" ? "your approval" : k;
}

export function linesFor(records: StepRecord[], stages: Stage[] = []): Line[] {
  const out: Line[] = [];
  // A heading when the first step of a stage starts, with the stage's why.
  stages.forEach((st, i) => {
    const starts = records.filter((r) => !r.parent && (st.steps ?? []).includes(r.stepId) && (r.startedAt || r.finishedAt)).map((r) => clock(r.startedAt ?? r.finishedAt).at);
    if (starts.length === 0) return;
    out.push({ at: Math.min(...starts) - 0.5, time: "", mark: "", text: `── ${i + 1}. ${st.name.toUpperCase()} ──${st.why ? `  ${st.why}` : ""}`, tone: "dim", head: true });
  });
  for (const r of records) {
    const name = r.name ?? r.stepId;
    const where = r.parent ? `${r.parent} › ` : "";
    const start = clock(r.startedAt);
    if (r.reusedFrom?.startsWith("keep:")) {
      out.push({ ...clock(r.finishedAt ?? r.startedAt), mark: "↻", text: `${where}${name} · kept from the last build of this stage (its sources have not changed)`, tone: "dim" });
      continue;
    }
    if (r.reusedFrom) {
      out.push({ ...clock(r.finishedAt ?? r.startedAt), mark: "↻", text: `${where}${name} · reused from an earlier run (nothing it depends on changed)`, tone: "dim" });
      continue;
    }
    if (r.edited) {
      out.push({ ...clock(r.finishedAt ?? r.startedAt), mark: "✎", text: `${where}${name} · your edit stands in for this step`, tone: "dim" });
      continue;
    }
    if (r.startedAt) {
      const model = r.model ? ` · ${shortModel(r.model)} · ${whereWord(r.model)}` : "";
      out.push({ ...start, mark: "▶", text: `${where}${name} · ${kindWord(r.kind)}${model}`, tone: r.status === "running" ? "bright" : "" });
    }
    for (const c of r.toolCalls ?? []) {
      out.push({ at: start.at + 1, time: "", mark: "", text: `${c.tool}  ${short(c.inputs)}  (${fmtMs(c.durationMs)})${c.error ? ` ✗ ${c.error}` : ""}`, tone: c.error ? "red" : "dim", indent: true });
    }
    if (r.status === "running") continue;
    const end = clock(r.finishedAt ?? r.startedAt);
    const tokens = r.promptTokens != null ? ` · ${fmtTokens(r.promptTokens)} → ${fmtTokens(r.completionTokens ?? 0)} tokens` : "";
    if (r.status === "failed") out.push({ ...end, mark: "✗", text: `${where}${name} · ${r.error ?? "failed"}`, tone: "red" });
    else if (r.status === "skipped" && r.kind === "review" && r.skipReason?.includes("decision")) out.push({ ...end, mark: "■", text: `${where}${name} · paused · needs you`, tone: "amber" });
    else if (r.status === "skipped") out.push({ ...end, mark: "·", text: `${where}${name} · skipped${r.skipReason ? ` · ${r.skipReason}` : ""}`, tone: "dim" });
    else out.push({ ...end, mark: "✓", text: `${where}${name} · ${fmtMs(r.durationMs)}${tokens}`, tone: "" });
  }
  return out.sort((a, b) => a.at - b.at);
}

function short(v: unknown): string {
  const s = typeof v === "string" ? v : JSON.stringify(v) ?? "";
  return s.length > 90 ? s.slice(0, 90) + "…" : s;
}

export function Teletype({ records, live, stages }: { records: StepRecord[]; live: boolean; stages?: Stage[] }) {
  const lines = useMemo(() => linesFor(records, stages ?? []), [records, stages]);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (live && ref.current) ref.current.scrollTop = ref.current.scrollHeight;
  }, [lines.length, live]);
  return (
    <div className="tty" ref={ref} role="log" aria-live={live ? "polite" : "off"}>
      {lines.length === 0 && <div className="dim">Waiting for the first step…</div>}
      {lines.map((l, i) => (
        <div key={i} className={`${l.indent ? "in" : ""} ${l.head ? "head" : ""} ${l.tone}`}>
          {!l.indent && !l.head && <span className="t">{l.time}</span>}{l.mark && <span className="mark">{l.mark} </span>}{l.text}
        </div>
      ))}
      {live && <div><span className="t">        </span><span className="cursor" /></div>}
    </div>
  );
}
