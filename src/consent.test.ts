import { describe, expect, it } from "vitest";
import { allTicked, allowedWord, consentOf, fmtBytes, grantKey, needsPermission, tickedGrants } from "./consent";
import type { Grant, RecipeInfo } from "./types";

const base: RecipeInfo = { id: "r", name: "R", version: null, description: null, source: "user", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [] };
const home: Grant = { kind: "read", entry: "~", text: "Read every file in your home folder." };
const net: Grant = { kind: "network", entry: "api.example.org", text: "Reach api.example.org." };

describe("consent", () => {
  it("an older engine, or a built-in, needs nothing", () => {
    expect(consentOf(base)).toEqual({ needed: [], granted: [] });
    expect(consentOf({ ...base, consent: null })).toEqual({ needed: [], granted: [] });
    expect(needsPermission(base)).toBe(false);
    expect(needsPermission({ ...base, consent: { needed: [home], granted: [] } })).toBe(true);
  });

  it("every needed line must be ticked; nothing needed is allowed", () => {
    expect(allTicked([], new Set())).toBe(true);
    expect(allTicked([home, net], new Set([grantKey(home)]))).toBe(false);
    expect(allTicked([home, net], new Set([grantKey(home), grantKey(net)]))).toBe(true);
    expect(tickedGrants([home, net], new Set([grantKey(net)]))).toEqual([net]);
  });

  it("says when a line was allowed, if the engine said", () => {
    expect(allowedWord(home)).toBe("allowed");
    expect(allowedWord({ ...home, grantedAt: "not a date" })).toBe("allowed");
    expect(allowedWord({ ...home, grantedAt: "2026-11-02T10:00:00Z" })).toMatch(/^allowed on /);
  });

  it("sizes in words", () => {
    expect(fmtBytes(512)).toBe("512 B");
    expect(fmtBytes(12 * 1024)).toBe("12 KB");
    expect(fmtBytes(1.5 * 1024 * 1024)).toBe("1.5 MB");
  });
});
