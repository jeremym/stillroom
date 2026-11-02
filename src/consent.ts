import type { Consent, Grant, RecipeInfo } from "./types";

/* The consent step, read without the engine. The engine decides which permission entries need a tick
   (anything that is not `input`, `output`, or `${inputs.x}`); this file only reads its answer. */

const NONE: Consent = { needed: [], granted: [] };

/** A recipe's consent state; an older engine that says nothing needs nothing. */
export function consentOf(r: RecipeInfo): Consent {
  const c = r.consent;
  if (!c) return NONE;
  return { needed: Array.isArray(c.needed) ? c.needed : [], granted: Array.isArray(c.granted) ? c.granted : [] };
}

/** One grant as a key: the same entry under the same kind is the same line. */
export function grantKey(g: Grant): string {
  return `${g.kind}:${g.entry}`;
}

/** True when every needed grant is among the ticked keys. Nothing needed is allowed. */
export function allTicked(needed: readonly Grant[], ticked: ReadonlySet<string>): boolean {
  return needed.every((g) => ticked.has(grantKey(g)));
}

/** The grants whose keys are ticked, in the order needed. */
export function tickedGrants(needed: readonly Grant[], ticked: ReadonlySet<string>): Grant[] {
  return needed.filter((g) => ticked.has(grantKey(g)));
}

/** True when the recipe cannot run until the user ticks something. */
export function needsPermission(r: RecipeInfo): boolean {
  return consentOf(r).needed.length > 0;
}

/** "allowed on 2 Nov 2026", or "allowed" when the engine did not say when. */
export function allowedWord(g: Grant): string {
  if (!g.grantedAt) return "allowed";
  const d = new Date(g.grantedAt);
  return Number.isNaN(d.getTime()) ? "allowed" : `allowed on ${d.toLocaleDateString()}`;
}

/** A size in words: 12 KB, 1.4 MB. */
export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
