import type { InputSpec, Permissions } from "./types";

/* Permission entries in words. `input` and `output` are the runtime's shorthands; `${inputs.x}`
   names one input; anything else is a path or a host. */

export function fsEntryLabel(entry: string, inputs: Record<string, InputSpec>): string {
  const m = entry.match(/^\$\{inputs\.([A-Za-z0-9_]+)\}$/);
  if (m) return inputs[m[1]]?.label ?? m[1];
  if (entry === "input") return "the files and folders you choose";
  if (entry === "output") return "the run's own folder";
  return entry;
}

export function netEntryLabel(entry: string, inputs: Record<string, InputSpec>): string {
  const m = entry.match(/^\$\{inputs\.([A-Za-z0-9_]+)\}$/);
  if (m) return `the sites in “${inputs[m[1]]?.label ?? m[1]}”`;
  if (entry === "input") return "the sites in the links you give it";
  if (entry === "*") return "any site";
  return entry;
}

/** One line: what a recipe may read, write, reach, and run. */
export function describePermissions(permissions: Permissions | null | undefined, inputs: Record<string, InputSpec>): string {
  const fs = permissions?.filesystem;
  const read = (fs?.read ?? ["input"]).map((e) => fsEntryLabel(e, inputs));
  const write = (fs?.write ?? []).map((e) => fsEntryLabel(e, inputs));
  const parts = [`reads ${read.join(", ") || "nothing"}`, `writes ${write.length ? write.join(", ") : "only its own results"}`];
  const net = permissions?.network ?? [];
  parts.push(net.length ? `may reach ${net.map((e) => netEntryLabel(e, inputs)).join(", ")}` : "no network");
  if (permissions?.shell) parts.push("may run programs");
  return parts.join(" · ");
}
