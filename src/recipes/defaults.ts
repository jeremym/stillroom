import type { InputSpec } from "../types";

/** The starting values a recipe declares (`default` on an input), as the fields hold them. A
    planned recipe keeps the links or notes it was made from here, so Set up starts filled in. */
export function defaultInputs(inputs: Record<string, InputSpec>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [name, spec] of Object.entries(inputs)) {
    const d = spec.default;
    if (d == null || d === "") continue;
    if (spec.type === "urls") {
      const links = (Array.isArray(d) ? d.map(String) : String(d).split("\n")).map((l) => l.trim()).filter(Boolean);
      if (links.length) out[name] = links;
    } else if (spec.type === "files") {
      if (Array.isArray(d) && d.length) out[name] = d.map(String);
    } else if (spec.type === "number") {
      if (typeof d === "number") out[name] = d;
    } else if (spec.type === "boolean") {
      out[name] = Boolean(d);
    } else if (spec.type === "json") {
      out[name] = d;
    } else {
      out[name] = typeof d === "string" ? d : JSON.stringify(d);
    }
  }
  return out;
}
