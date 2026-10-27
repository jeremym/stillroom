import type { Edge, Node } from "@xyflow/react";
import { fmtMs } from "../api";
import type { InputSpec, OutputSpec, RecipeInfo, Run, StepInfo, StepRecord } from "../types";

/* Flow View's data layer: a recipe (and, when there is one, a run of it) becomes a graph of
   positioned nodes and edges. Pure functions, no React. Inputs on the left, results on the right,
   steps in between by how far they are from the inputs. A map (for each) is a box with its inner
   steps inside. */

export type NodeStatus =
  | "none" | "waiting" | "running" | "done" | "reused" | "edited" | "skipped" | "failed" | "needs_you" | "held";

export interface FlowNodeData extends Record<string, unknown> {
  role: "input" | "step" | "output" | "group";
  /** Human title: the step name, the input label, the output label. */
  title: string;
  /** What kind of thing this is, in plain words ("Tool", "AI step", "Agent", "For each", "Your approval"). */
  kindLabel: string;
  /** Second line: the tool id, the model, the agent's tools, the output type. */
  subtitle: string;
  status: NodeStatus;
  statusText: string;
  /** "only if …" line for conditional steps. */
  condition?: string;
  stepId?: string;
  inputName?: string;
  outputName?: string;
  /** The step kind (tool, model, agent, map, review) for the glyph. */
  kind?: string;
  /** The input type or output type, for the glyph on an input or result. */
  ioType?: string;
  /** When the step started, for a live timer while it runs. */
  startedAt?: string | null;
}

export type FlowNode = Node<FlowNodeData>;

const NODE_W = 236;
const NODE_H = 96;
const IO_W = 196;
const IO_H = 92;
const ROW_GAP = 56;
const COL_GAP = 40;
const LANE_GAP = 56;
const STACK_GAP = 14;
const INNER_GAP_X = 44;
const INNER_GAP_Y = 24;
const GROUP_PAD = 16;
const GROUP_MIN_W = 340;
const GROUP_HEAD = 72;

export const KIND_LABEL: Record<string, string> = {
  tool: "Tool",
  model: "AI step",
  agent: "Agent",
  map: "For each",
  review: "Your approval",
  recipe: "Recipe",
  branch: "Branch",
};

export const KIND_EXPLAINED: Record<string, string> = {
  tool: "A fixed program step: it always does the same thing with its inputs, no model involved.",
  model: "One prompt to a language model, with the answer checked against a schema (a fixed shape).",
  agent: "A model in a loop that can call the listed tools, decide what to do next, then answer.",
  map: "Runs its inner steps once for each item in a list (a for-each loop).",
  review: "Pauses the run and waits for you. Only what you approve goes on.",
};

export const STATUS_WORD: Record<NodeStatus, string> = {
  none: "",
  waiting: "waiting",
  running: "running…",
  done: "done",
  reused: "reused",
  edited: "edited by you",
  skipped: "skipped",
  failed: "failed",
  needs_you: "needs you",
  held: "waiting for your decision",
};

export const INPUT_TYPE_WORD: Record<InputSpec["type"], string> = {
  string: "text", text: "text", number: "number", boolean: "yes/no", file: "file", files: "files",
  url: "link", urls: "links", directory: "folder", json: "data",
};

export function findStep(steps: StepInfo[], id: string): StepInfo | undefined {
  for (const s of steps) {
    if (s.id === id) return s;
    const inner = s.steps && findStep(s.steps, id);
    if (inner) return inner;
  }
  return undefined;
}

/** The map step that contains `innerId`, if any. */
export function parentOf(steps: StepInfo[], innerId: string): StepInfo | undefined {
  return steps.find((s) => s.steps?.some((i) => i.id === innerId));
}

/** `${steps.x.output.a.b}` → "Sort it → a.b"; `${inputs.x}` → the input's label; `${email.path}` → "each email's path". */
export function describeRef(ref: string, recipe: RecipeInfo, itemName?: string): string {
  const m = ref.match(/^\$\{([A-Za-z_][A-Za-z0-9_]*)((?:\.[A-Za-z0-9_]+)*)\}$/);
  if (!m) return ref;
  const parts = m[2].split(".").filter(Boolean);
  if (m[1] === "steps") {
    const step = findStep(recipe.steps, parts[0]);
    const rest = parts.slice(2);
    return `${step?.name ?? parts[0]}${rest.length ? " → " + rest.join(".") : ""}`;
  }
  if (m[1] === "inputs") {
    return `${recipe.inputs[parts[0]]?.label ?? parts[0]}${parts.length > 1 ? " → " + parts.slice(1).join(".") : ""}`;
  }
  return `each ${itemName ?? m[1]}${parts.length ? "'s " + parts.join(".") : ""}`;
}

export function sourceStepOf(spec: OutputSpec): string | null {
  const m = spec.from.match(/^\$\{steps\.([A-Za-z0-9_-]+)\.output/);
  return m ? m[1] : null;
}

/** Records for a step: one for a top-level step, one per item for a step inside a map. */
export function recordsFor(records: StepRecord[], stepId: string, parentId?: string): StepRecord[] {
  if (!parentId) return records.filter((r) => r.stepId === stepId);
  const re = new RegExp(`^${escapeRe(parentId)}\\[\\d+\\]\\.${escapeRe(stepId)}$`);
  return records.filter((r) => re.test(r.stepId)).sort((a, b) => a.position - b.position);
}

function escapeRe(s: string) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function recordStatus(r: StepRecord): NodeStatus {
  if (r.edited) return "edited";
  if (r.reusedFrom) return "reused";
  if (r.status === "running") return "running";
  if (r.status === "succeeded") return "done";
  if (r.status === "failed") return "failed";
  if (r.status === "pending") return "waiting";
  if (r.kind === "review" && r.skipReason?.includes("decision")) return "needs_you";
  if (r.skipReason === "waiting for review") return "held";
  return "skipped";
}

const PRIORITY: NodeStatus[] = ["running", "failed", "needs_you", "held", "waiting", "done", "edited", "reused", "skipped", "none"];

/** One status and one line of text for a node, from however many records it has. */
export function summarize(records: StepRecord[], inProgress: boolean): { status: NodeStatus; text: string; startedAt?: string | null } {
  if (records.length === 0) return inProgress ? { status: "waiting", text: "waiting" } : { status: "none", text: "" };
  if (records.length === 1) {
    const r = records[0];
    const status = recordStatus(r);
    const time = r.durationMs != null && status !== "reused" && status !== "edited" && r.durationMs > 0 ? ` · ${fmtMs(r.durationMs)}` : "";
    const reason = status === "skipped" && r.skipReason ? ` · ${r.skipReason}` : "";
    return { status, text: `${STATUS_WORD[status]}${time}${reason}`, startedAt: status === "running" ? r.startedAt : undefined };
  }
  const running = records.find((r) => r.status === "running");
  const counts = new Map<NodeStatus, number>();
  for (const r of records) {
    const s = recordStatus(r);
    counts.set(s, (counts.get(s) ?? 0) + 1);
  }
  const status = PRIORITY.find((s) => counts.has(s)) ?? "none";
  const text = PRIORITY.filter((s) => counts.has(s)).map((s) => `${counts.get(s)} ${STATUS_WORD[s]}`).join(" · ");
  return { status, text, startedAt: running?.startedAt };
}

function stepSubtitle(s: StepInfo, modelFor: (slot: string) => string | null): string {
  if (s.kind === "tool") return s.tool ?? "";
  if (s.kind === "model") return modelFor(s.slot ?? "") ?? `${s.slot} (auto)`;
  if (s.kind === "agent") return `${modelFor(s.slot ?? "") ?? `${s.slot} (auto)`} · ${s.allowedTools?.join(", ") || "no tools"}`;
  if (s.kind === "map") return `${s.steps?.length ?? 0} inner step${s.steps?.length === 1 ? "" : "s"} per item`;
  if (s.kind === "review") return s.title ?? "approve or reject each item";
  return s.kind;
}

export interface GraphOptions {
  recipe: RecipeInfo;
  run: Run | null;
  /** Live records while a run is in progress (or the finished run's records). */
  records: StepRecord[];
  inProgress: boolean;
  /** Model name to show for a slot: the run's resolution, else the pin, else null (auto). */
  modelFor: (slot: string) => string | null;
  selectedId: string | null;
}

export interface Graph {
  nodes: FlowNode[];
  edges: Edge[];
  /** Size of the whole drawing, so the canvas can be sized to show it at full scale. */
  width: number;
  height: number;
}

/* Layout: steps flow top to bottom, one row per depth (a step sits one row below the deepest
   step it reads from). Inputs sit in a lane on the left beside the first row that reads them;
   results sit in a lane on the right beside the step that produces them. So every line is either
   a short horizontal into or out of a step, or a vertical from one row to the next. */
export function buildGraph({ recipe, run, records, inProgress, modelFor, selectedId }: GraphOptions): Graph {
  const steps = recipe.steps;
  const stepIds = new Set(steps.map((s) => s.id));

  const depth = new Map<string, number>();
  const visiting = new Set<string>();
  const depthOf = (id: string): number => {
    const known = depth.get(id);
    if (known != null) return known;
    if (visiting.has(id)) return 1;
    visiting.add(id);
    const s = steps.find((x) => x.id === id);
    const deps = (s?.dependsOn ?? []).filter((d) => stepIds.has(d));
    const d = deps.length ? 1 + Math.max(...deps.map(depthOf)) : 1;
    depth.set(id, d);
    visiting.delete(id);
    return d;
  };
  steps.forEach((s) => depthOf(s.id));
  const maxDepth = Math.max(1, ...depth.values());

  interface Placed { lane: "in" | "center" | "out"; row: number; w: number; h: number; node: FlowNode; children?: FlowNode[] }
  const placed: Placed[] = [];
  const edges: Edge[] = [];

  for (const [name, spec] of Object.entries(recipe.inputs)) {
    const value = run?.inputs?.[name];
    const shown = typeof value === "string" ? value.split("/").filter(Boolean).pop() ?? value : Array.isArray(value) ? `${value.length} chosen` : value != null ? String(value) : "";
    const consumers = steps.filter((s) => s.usesInputs?.includes(name)).map((s) => depth.get(s.id) ?? 1);
    placed.push({
      lane: "in", row: consumers.length ? Math.min(...consumers) : 1, w: IO_W, h: IO_H,
      node: {
        id: `in:${name}`, type: "io", position: { x: 0, y: 0 },
        data: {
          role: "input", title: spec.label ?? name, kindLabel: `Input · ${INPUT_TYPE_WORD[spec.type] ?? spec.type}`,
          subtitle: shown || (spec.required === false ? "optional" : ""), status: "none", statusText: "", inputName: name, ioType: spec.type,
        },
        selected: selectedId === `in:${name}`,
      },
    });
  }

  for (const s of steps) {
    const row = depth.get(s.id) ?? 1;
    const id = `step:${s.id}`;
    const recs = recordsFor(records, s.id);
    const { status, text, startedAt } = summarize(recs, inProgress);
    const base: FlowNodeData = {
      role: "step", title: s.name ?? s.id, kindLabel: KIND_LABEL[s.kind] ?? s.kind, subtitle: stepSubtitle(s, modelFor),
      status, statusText: text, stepId: s.id, kind: s.kind, startedAt,
      condition: s.when ? `only if ${describeRef(s.when, recipe)}` : undefined,
    };
    for (const d of s.dependsOn) if (stepIds.has(d)) edges.push(edge(`step:${d}`, id, status, "b", "t"));
    for (const i of s.usesInputs ?? []) if (recipe.inputs[i]) edges.push(edge(`in:${i}`, id, status, "r", "l"));

    if (s.kind === "map" && s.steps?.length) {
      const inner = s.steps;
      const innerIds = new Set(inner.map((i) => i.id));
      const innerDepth = new Map<string, number>();
      const dOf = (iid: string): number => {
        const k = innerDepth.get(iid);
        if (k != null) return k;
        const st = inner.find((x) => x.id === iid);
        const deps = (st?.dependsOn ?? []).filter((d) => innerIds.has(d) && d !== iid);
        innerDepth.set(iid, 0);
        const d = deps.length ? 1 + Math.max(...deps.map(dOf)) : 0;
        innerDepth.set(iid, d);
        return d;
      };
      inner.forEach((i) => dOf(i.id));
      const cols = Math.max(...innerDepth.values()) + 1;
      const rowsPerCol = new Map<number, number>();
      const children: FlowNode[] = [];
      const over = (recs[0]?.inputs as { over?: unknown[] } | undefined)?.over;
      const itemCount = Array.isArray(over) ? over.length : null;
      for (const i of inner) {
        const c = innerDepth.get(i.id) ?? 0;
        const r = rowsPerCol.get(c) ?? 0;
        rowsPerCol.set(c, r + 1);
        const cid = `step:${s.id}/${i.id}`;
        const irecs = recordsFor(records, i.id, s.id);
        const sum = irecs.length === 0 && (status === "reused" || status === "edited")
          ? { status, text: `${STATUS_WORD[status]} with the loop` }
          : summarize(irecs, inProgress && recs.length > 0);
        children.push({
          id: cid, type: "step", parentId: id, extent: "parent",
          position: { x: GROUP_PAD + c * (NODE_W + INNER_GAP_X), y: GROUP_HEAD + r * (NODE_H + INNER_GAP_Y) },
          data: {
            role: "step", title: i.name ?? i.id, kindLabel: KIND_LABEL[i.kind] ?? i.kind, subtitle: stepSubtitle(i, modelFor),
            status: sum.status, statusText: sum.text, stepId: i.id, kind: i.kind, startedAt: sum.startedAt,
            condition: i.when ? `only if ${describeRef(i.when, recipe, itemName(s))}` : undefined,
          },
          selected: selectedId === cid,
          draggable: false,
        });
        for (const d of i.dependsOn) {
          if (innerIds.has(d)) edges.push(edge(`step:${s.id}/${d}`, cid, sum.status, "r", "l"));
          else if (stepIds.has(d)) edges.push(edge(`step:${d}`, cid, sum.status, "b", "t"));
        }
      }
      const rows = Math.max(...rowsPerCol.values());
      const innerW = cols * NODE_W + (cols - 1) * INNER_GAP_X;
      const w = Math.max(GROUP_MIN_W, GROUP_PAD * 2 + innerW);
      for (const c of children) c.position.x += (w - GROUP_PAD * 2 - innerW) / 2;
      const h = GROUP_HEAD + rows * NODE_H + (rows - 1) * INNER_GAP_Y + GROUP_PAD;
      placed.push({
        lane: "center", row, w, h, children,
        node: {
          id, type: "group", position: { x: 0, y: 0 }, style: { width: w, height: h },
          data: { ...base, subtitle: itemCount != null ? `${itemCount} item${itemCount === 1 ? "" : "s"} · ${base.subtitle}` : base.subtitle },
          selected: selectedId === id,
        },
      });
    } else {
      placed.push({ lane: "center", row, w: NODE_W, h: NODE_H, node: { id, type: "step", position: { x: 0, y: 0 }, data: base, selected: selectedId === id } });
    }
  }

  for (const [name, spec] of Object.entries(recipe.outputs)) {
    const id = `out:${name}`;
    const src = sourceStepOf(spec);
    const srcStatus = src ? summarize(recordsFor(records, src), inProgress).status : "none";
    const produced = run?.outputs?.[name] != null;
    placed.push({
      lane: "out", row: src ? depth.get(src) ?? maxDepth : maxDepth, w: IO_W, h: IO_H,
      node: {
        id, type: "io", position: { x: 0, y: 0 },
        data: {
          role: "output", title: spec.label ?? name, kindLabel: `Result · ${spec.type}`,
          subtitle: src ? `from ${findStep(steps, src)?.name ?? src}` : "",
          status: produced ? "done" : srcStatus === "none" ? "none" : "waiting", statusText: produced ? "ready" : "", outputName: name, ioType: spec.type,
        },
        selected: selectedId === id,
      },
    });
    if (src && stepIds.has(src)) edges.push(edge(`step:${src}`, id, srcStatus, "r", "l"));
  }

  // Rows: a row is as tall as its tallest step or its tallest stack of inputs/results.
  const inRow = (lane: Placed["lane"], r: number) => placed.filter((p) => p.lane === lane && p.row === r);
  const stackH = (items: Placed[]) => items.reduce((h, p, i) => h + p.h + (i ? STACK_GAP : 0), 0);
  const rowW = (items: Placed[]) => items.reduce((w, p, i) => w + p.w + (i ? COL_GAP : 0), 0);
  const centerW = Math.max(NODE_W, ...Array.from({ length: maxDepth }, (_, i) => rowW(inRow("center", i + 1))));
  const hasIn = placed.some((p) => p.lane === "in");
  const hasOut = placed.some((p) => p.lane === "out");
  const centerX = hasIn ? IO_W + LANE_GAP : 0;
  const outX = centerX + centerW + LANE_GAP;
  // Where each top-level step sits in its row, for routing result lines around siblings.
  const rowOf = new Map<string, { bottom: number; index: number; count: number }>();
  let y = 0;
  for (let r = 1; r <= maxDepth; r++) {
    const ins = inRow("in", r);
    const mids = inRow("center", r);
    const outs = inRow("out", r);
    const rowH = Math.max(stackH(ins), stackH(outs), ...mids.map((p) => p.h), mids.length ? 0 : IO_H);
    let x = centerX + (centerW - rowW(mids)) / 2;
    mids.forEach((p, index) => {
      p.node.position = { x, y: y + (rowH - p.h) / 2 };
      x += p.w + COL_GAP;
      rowOf.set(p.node.id, { bottom: y + (rowH + p.h) / 2, index, count: mids.length });
    });
    let sy = y + (rowH - stackH(ins)) / 2;
    for (const p of ins) {
      p.node.position = { x: 0, y: sy };
      sy += p.h + STACK_GAP;
    }
    sy = y + (rowH - stackH(outs)) / 2;
    for (const p of outs) {
      p.node.position = { x: outX, y: sy };
      sy += p.h + STACK_GAP;
    }
    y += rowH + ROW_GAP;
  }

  // A result line from a step that is not the rightmost in its row would cross its siblings;
  // route it down from the step, along a bus under the row, then across to the result.
  for (const e of edges) {
    const row = rowOf.get(e.source);
    if (e.target.startsWith("out:") && row && row.count > 1 && row.index < row.count - 1) {
      e.type = "bus";
      e.sourceHandle = "b";
      e.data = { viaY: row.bottom + 18 + 10 * row.index };
    }
  }

  // Every node carries its size, so React Flow draws it at once instead of waiting to
  // measure it (a measurement the webview sometimes never delivers).
  const nodes: FlowNode[] = [];
  for (const p of placed) {
    nodes.push({ ...p.node, width: p.w, height: p.h });
    if (p.children) nodes.push(...p.children.map((c) => ({ ...c, width: NODE_W, height: NODE_H })));
  }
  return { nodes, edges: dedupe(edges), width: hasOut ? outX + IO_W : centerX + centerW, height: Math.max(0, y - ROW_GAP) };
}

function itemName(map: StepInfo): string | undefined {
  return map.itemName ?? "item";
}

function edge(source: string, target: string, targetStatus: NodeStatus, sourceHandle: string, targetHandle: string): Edge {
  return {
    id: `${source}->${target}`, source, target, sourceHandle, targetHandle, type: "smoothstep",
    animated: targetStatus === "running",
    className: `flow-edge ${targetStatus}`,
  };
}

function dedupe(edges: Edge[]): Edge[] {
  const seen = new Set<string>();
  return edges.filter((e) => (seen.has(e.id) ? false : (seen.add(e.id), true)));
}
