import type { InputSpec, OutputSpec, RecipeDoc, RecipeInfo, StepDoc, StepInfo } from "../types";

/* Editing a recipe: pure functions over the recipe.yaml document. Every function returns a new
   document; nothing here touches the disk. Node ids match graph.ts: in:<name>, out:<name>,
   step:<id>, step:<map>/<inner>. */

export const STEP_KINDS = ["tool", "model", "agent", "map", "review"] as const;
export type StepKind = (typeof STEP_KINDS)[number];

const REF = /^\$\{([A-Za-z_][A-Za-z0-9_]*)((?:\.[A-Za-z0-9_]+)*)\}$/;

export function parseRef(s: unknown): { root: string; parts: string[] } | null {
  if (typeof s !== "string") return null;
  const m = s.match(REF);
  return m ? { root: m[1], parts: m[2].split(".").filter(Boolean) } : null;
}

/** Every whole-string reference anywhere inside `v`. */
export function refsIn(v: unknown, out: string[] = []): string[] {
  if (typeof v === "string") {
    if (REF.test(v)) out.push(v);
  } else if (Array.isArray(v)) v.forEach((x) => refsIn(x, out));
  else if (v && typeof v === "object") Object.values(v).forEach((x) => refsIn(x, out));
  return out;
}

function ownRefs(step: StepDoc): string[] {
  const out = refsIn(step.inputs ?? {});
  for (const s of [step.when, step.over, step.items]) if (typeof s === "string" && REF.test(s)) out.push(s);
  return out;
}

function stepIdsIn(refs: string[]): string[] {
  const ids = new Set<string>();
  for (const r of refs) {
    const p = parseRef(r);
    if (p?.root === "steps" && p.parts[0]) ids.add(p.parts[0]);
  }
  return [...ids];
}

function inputNamesIn(refs: string[]): string[] {
  const names = new Set<string>();
  for (const r of refs) {
    const p = parseRef(r);
    if (p?.root === "inputs" && p.parts[0]) names.add(p.parts[0]);
  }
  return [...names];
}

/** What the graph needs about each step, derived from the document the same way the Rust side does. */
export function stepInfosFromDoc(doc: RecipeDoc): StepInfo[] {
  const info = (s: StepDoc, siblings: Set<string> | null): StepInfo => {
    const refs = ownRefs(s);
    let deps = new Set([...stepIdsIn(refs), ...(s.dependsOn ?? [])]);
    let inputs = new Set(inputNamesIn(refs));
    const inner = s.kind === "map" ? (s.steps ?? []) : [];
    if (inner.length) {
      const innerIds = new Set(inner.map((i) => i.id));
      for (const i of inner) {
        const r = ownRefs(i);
        for (const d of [...stepIdsIn(r), ...(i.dependsOn ?? [])]) if (!innerIds.has(d)) deps.add(d);
        for (const n of inputNamesIn(r)) inputs.add(n);
      }
    }
    if (siblings) deps = new Set([...deps].filter((d) => d !== s.id));
    return {
      id: s.id,
      kind: s.kind,
      name: s.name ?? null,
      slot: s.slot ?? null,
      tool: s.tool ?? null,
      dependsOn: [...deps].sort(),
      usesInputs: [...inputs].sort(),
      when: s.when ?? null,
      steps: inner.length ? inner.map((i) => info(i, new Set(inner.map((x) => x.id)))) : undefined,
      itemName: s.kind === "map" ? s.as ?? "item" : undefined,
      allowedTools: s.allowedTools,
      show: s.show,
      title: s.title ?? null,
    };
  };
  return doc.steps.map((s) => info(s, null));
}

/** The document as a RecipeInfo, so the graph can draw it. */
export function docToInfo(doc: RecipeDoc, base: RecipeInfo): RecipeInfo {
  return {
    ...base,
    id: doc.id,
    name: doc.name,
    version: doc.version ?? null,
    description: doc.description ?? null,
    inputs: doc.inputs ?? {},
    modelSlots: doc.modelSlots ?? {},
    permissions: doc.permissions ?? null,
    steps: stepInfosFromDoc(doc),
    outputs: doc.outputs ?? {},
  };
}

export interface Located { step: StepDoc; parent: StepDoc | null; list: StepDoc[]; index: number }

/** Find a step by node path: "id" or "map/inner". */
export function locate(doc: RecipeDoc, path: string): Located | null {
  const [a, b] = path.split("/");
  const i = doc.steps.findIndex((s) => s.id === a);
  if (i < 0) return null;
  if (!b) return { step: doc.steps[i], parent: null, list: doc.steps, index: i };
  const parent = doc.steps[i];
  const list = parent.steps ?? [];
  const j = list.findIndex((s) => s.id === b);
  return j < 0 ? null : { step: list[j], parent, list, index: j };
}

export function mutate(doc: RecipeDoc, fn: (d: RecipeDoc) => void): RecipeDoc {
  const next = structuredClone(doc);
  fn(next);
  return next;
}

export function updateStep(doc: RecipeDoc, path: string, patch: Partial<StepDoc>): RecipeDoc {
  return mutate(doc, (d) => {
    const loc = locate(d, path);
    if (!loc) return;
    for (const [k, v] of Object.entries(patch)) {
      if (v === undefined) delete loc.step[k];
      else loc.step[k] = v;
    }
  });
}

export function uniqueId(base: string, taken: Iterable<string>): string {
  const set = new Set(taken);
  let id = base.replace(/[^A-Za-z0-9_]/g, "_").replace(/^[^A-Za-z]+/, "") || "step";
  let n = 2;
  const root = id;
  while (set.has(id)) id = `${root}_${n++}`;
  return id;
}

export function allStepIds(doc: RecipeDoc): string[] {
  return doc.steps.flatMap((s) => [s.id, ...(s.steps ?? []).map((i) => i.id)]);
}

/** A new step of `kind` with sensible defaults. Model and agent steps get a prompt file path. */
export function newStep(doc: RecipeDoc, kind: StepKind): StepDoc {
  const id = uniqueId(kind, allStepIds(doc));
  const slot = Object.keys(doc.modelSlots ?? {})[0] ?? "writer";
  switch (kind) {
    case "tool": return { id, kind, name: "New tool step", tool: "", inputs: {} };
    case "model": return { id, kind, name: "New AI step", slot, prompt: `prompts/${id}.md`, inputs: {} };
    case "agent": return { id, kind, name: "New agent", slot, prompt: `prompts/${id}.md`, allowedTools: [], inputs: {} };
    case "map": return { id, kind, name: "For each item", over: "", as: "item", steps: [{ id: "first", kind: "tool", name: "First inner step", tool: "", inputs: {} }] };
    case "review": return { id, kind, name: "Your approval", items: "", title: "Items to approve", show: [] };
  }
}

/** Add a step at the end of the recipe, or inside the map `parentId`. Returns the new doc and the node path. */
export function addStep(doc: RecipeDoc, kind: StepKind, parentId?: string): { doc: RecipeDoc; path: string } {
  const step = newStep(doc, kind);
  const next = mutate(doc, (d) => {
    if (parentId) {
      const parent = d.steps.find((s) => s.id === parentId);
      if (parent && parent.kind === "map") (parent.steps ??= []).push(step);
    } else d.steps.push(step);
  });
  return { doc: next, path: parentId ? `${parentId}/${step.id}` : step.id };
}

export function removeStep(doc: RecipeDoc, path: string): RecipeDoc {
  return mutate(doc, (d) => {
    const loc = locate(d, path);
    if (loc) loc.list.splice(loc.index, 1);
  });
}

/** Steps (by node path) that read from the step at `path`. */
export function dependentsOf(doc: RecipeDoc, path: string): string[] {
  const loc = locate(doc, path);
  if (!loc) return [];
  const id = loc.step.id;
  const reads = (s: StepDoc) => stepIdsIn(ownRefs(s)).includes(id) || (s.dependsOn ?? []).includes(id);
  const out: string[] = [];
  if (loc.parent) {
    for (const s of loc.list) if (s.id !== id && reads(s)) out.push(`${loc.parent.id}/${s.id}`);
  } else {
    for (const s of doc.steps) {
      if (s.id !== id && reads(s)) out.push(s.id);
      for (const i of s.steps ?? []) if (reads(i)) out.push(`${s.id}/${i.id}`);
    }
    for (const [name, o] of Object.entries(doc.outputs ?? {})) {
      const p = parseRef(o.from);
      if (p?.root === "steps" && p.parts[0] === id) out.push(`out:${name}`);
    }
  }
  return out;
}

/** Rename a step id everywhere: the step, references to it, dependsOn lists, and results. */
export function renameStepId(doc: RecipeDoc, path: string, newId: string): RecipeDoc {
  const loc = locate(doc, path);
  if (!loc || !/^[A-Za-z][A-Za-z0-9_]*$/.test(newId) || newId === loc.step.id) return doc;
  const old = loc.step.id;
  const swap = (v: unknown): unknown => {
    if (typeof v === "string") {
      if (v === `\${steps.${old}.output}`) return `\${steps.${newId}.output}`;
      if (v.startsWith(`\${steps.${old}.output.`)) return `\${steps.${newId}.output.` + v.slice(`\${steps.${old}.output.`.length);
      return v;
    }
    if (Array.isArray(v)) return v.map(swap);
    if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, swap(x)]));
    return v;
  };
  const fix = (s: StepDoc) => {
    if (s.inputs) s.inputs = swap(s.inputs) as Record<string, unknown>;
    for (const k of ["when", "over", "items"] as const) if (typeof s[k] === "string") s[k] = swap(s[k]) as string;
    if (s.dependsOn) s.dependsOn = s.dependsOn.map((d) => (d === old ? newId : d));
  };
  return mutate(doc, (d) => {
    const l = locate(d, path)!;
    l.step.id = newId;
    const scope = l.parent ? l.list : d.steps;
    for (const s of scope) {
      fix(s);
      if (!l.parent) for (const i of s.steps ?? []) fix(i);
    }
    if (!l.parent) for (const o of Object.values(d.outputs ?? {})) o.from = swap(o.from) as string;
  });
}

/** Reference text for a node used as a source, from the point of view of the target step. */
export function refFor(sourceNodeId: string): string | null {
  if (sourceNodeId.startsWith("in:")) return `\${inputs.${sourceNodeId.slice(3)}}`;
  if (sourceNodeId.startsWith("step:")) {
    const id = sourceNodeId.slice(5).split("/").pop()!;
    return `\${steps.${id}.output}`;
  }
  return null;
}

export function stepPathOf(nodeId: string): string | null {
  return nodeId.startsWith("step:") ? nodeId.slice(5) : null;
}

/** Wire `source` into `target`: a step gets it under `key`; a result takes it as its `from`. */
export function connect(doc: RecipeDoc, sourceNodeId: string, targetNodeId: string, key?: string): RecipeDoc {
  const ref = refFor(sourceNodeId);
  if (!ref) return doc;
  if (targetNodeId.startsWith("out:")) {
    const name = targetNodeId.slice(4);
    return mutate(doc, (d) => {
      if (d.outputs?.[name]) d.outputs[name].from = ref;
    });
  }
  const path = stepPathOf(targetNodeId);
  if (!path || !key) return doc;
  return mutate(doc, (d) => {
    const loc = locate(d, path);
    if (!loc) return;
    if (key === "over" && loc.step.kind === "map") loc.step.over = ref;
    else if (key === "items" && loc.step.kind === "review") loc.step.items = ref;
    else if (key === "when") loc.step.when = ref;
    else (loc.step.inputs ??= {})[key] = ref;
  });
}

/** Undo every reference from `source` inside `target`. Removed inputs vanish; over/items/from become empty. */
export function disconnect(doc: RecipeDoc, sourceNodeId: string, targetNodeId: string): RecipeDoc {
  const ref = refFor(sourceNodeId);
  if (!ref) return doc;
  const base = ref.slice(0, -1); // "${steps.x.output" or "${inputs.x"
  const hits = (v: unknown) => typeof v === "string" && (v === ref || v.startsWith(base + "."));
  if (targetNodeId.startsWith("out:")) {
    const name = targetNodeId.slice(4);
    return mutate(doc, (d) => {
      if (d.outputs?.[name] && hits(d.outputs[name].from)) d.outputs[name].from = "";
    });
  }
  const path = stepPathOf(targetNodeId);
  if (!path) return doc;
  const sourceStep = sourceNodeId.startsWith("step:") ? sourceNodeId.slice(5).split("/").pop() : null;
  return mutate(doc, (d) => {
    const loc = locate(d, path);
    if (!loc) return;
    const s = loc.step;
    for (const [k, v] of Object.entries(s.inputs ?? {})) if (hits(v) || refsIn(v).some(hits)) delete s.inputs![k];
    if (hits(s.when)) delete s.when;
    if (hits(s.over)) s.over = "";
    if (hits(s.items)) s.items = "";
    if (sourceStep && s.dependsOn) s.dependsOn = s.dependsOn.filter((x) => x !== sourceStep);
    if (s.dependsOn?.length === 0) delete s.dependsOn;
  });
}

export interface Source { ref: string; label: string }

/** What a step at `path` may refer to: inputs, steps it can see, and the loop item if inside a map. */
export function sourcesFor(doc: RecipeDoc, path: string | null): Source[] {
  const out: Source[] = [];
  for (const [name, spec] of Object.entries(doc.inputs ?? {})) out.push({ ref: `\${inputs.${name}}`, label: `Input: ${spec.label ?? name}` });
  const loc = path ? locate(doc, path) : null;
  const self = loc?.step.id;
  const exclude = loc?.parent ? loc.parent.id : self;
  for (const s of doc.steps) if (s.id !== exclude) out.push({ ref: `\${steps.${s.id}.output}`, label: `Step: ${s.name ?? s.id}` });
  if (loc?.parent) {
    out.push({ ref: `\${${loc.parent.as ?? "item"}}`, label: `Each ${loc.parent.as ?? "item"} of the loop` });
    for (const s of loc.list) if (s.id !== self) out.push({ ref: `\${steps.${s.id}.output}`, label: `Inner step: ${s.name ?? s.id}` });
  }
  return out;
}

export function renameKey<T>(obj: Record<string, T>, from: string, to: string): Record<string, T> {
  if (from === to || !to || to in obj) return obj;
  return Object.fromEntries(Object.entries(obj).map(([k, v]) => [k === from ? to : k, v]));
}

export function newInput(doc: RecipeDoc): { name: string; spec: InputSpec } {
  return { name: uniqueId("input", Object.keys(doc.inputs ?? {})), spec: { type: "text", label: "New input" } };
}

export function newOutput(doc: RecipeDoc): { name: string; spec: OutputSpec } {
  const last = doc.steps[doc.steps.length - 1];
  return { name: uniqueId("result", Object.keys(doc.outputs ?? {})), spec: { from: last ? `\${steps.${last.id}.output}` : "", type: "json", label: "New result" } };
}

/** Rename an input everywhere: the declaration and every `${inputs.old…}` reference. */
export function renameInput(doc: RecipeDoc, from: string, to: string): RecipeDoc {
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(to) || from === to || doc.inputs?.[to]) return doc;
  const swap = (v: unknown): unknown => {
    if (typeof v === "string") {
      if (v === `\${inputs.${from}}`) return `\${inputs.${to}}`;
      if (v.startsWith(`\${inputs.${from}.`)) return `\${inputs.${to}.` + v.slice(`\${inputs.${from}.`.length);
      return v;
    }
    if (Array.isArray(v)) return v.map(swap);
    if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, swap(x)]));
    return v;
  };
  return mutate(doc, (d) => {
    d.inputs = renameKey(d.inputs ?? {}, from, to);
    const fix = (s: StepDoc) => {
      if (s.inputs) s.inputs = swap(s.inputs) as Record<string, unknown>;
      for (const k of ["when", "over", "items"] as const) if (typeof s[k] === "string") s[k] = swap(s[k]) as string;
    };
    for (const s of d.steps) {
      fix(s);
      for (const i of s.steps ?? []) fix(i);
    }
    if (d.permissions?.filesystem) {
      const fs = d.permissions.filesystem;
      fs.read = fs.read?.map((x) => swap(x) as string);
      fs.write = fs.write?.map((x) => swap(x) as string);
    }
    for (const o of Object.values(d.outputs ?? {})) o.from = swap(o.from) as string;
  });
}

/** Rename a model slot everywhere: the declaration and every step that uses it. */
export function renameSlot(doc: RecipeDoc, from: string, to: string): RecipeDoc {
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(to) || from === to || doc.modelSlots?.[to]) return doc;
  return mutate(doc, (d) => {
    d.modelSlots = renameKey(d.modelSlots ?? {}, from, to);
    for (const s of d.steps) {
      if (s.slot === from) s.slot = to;
      for (const i of s.steps ?? []) if (i.slot === from) i.slot = to;
    }
  });
}
