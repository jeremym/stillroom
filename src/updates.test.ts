import { describe, expect, it } from "vitest";
import { agoWord, updateWord } from "./updates";
import type { UpdateCheck } from "./types";

const NOW = new Date("2026-11-04T12:00:00Z");
const check = (extra: Partial<UpdateCheck> = {}): UpdateCheck => ({ current: "0.3.3", latest: "0.3.3", newer: false, url: null, notes: null, publishedAt: null, checkedAt: "2026-11-04T11:57:00Z", error: null, ...extra });

describe("agoWord", () => {
  it("rounds to the unit a person would say", () => {
    expect(agoWord("2026-11-04T11:59:30Z", NOW)).toBe("just now");
    expect(agoWord("2026-11-04T11:40:00Z", NOW)).toBe("20 min ago");
    expect(agoWord("2026-11-04T09:00:00Z", NOW)).toBe("3 hours ago");
    expect(agoWord("2026-11-03T10:00:00Z", NOW)).toBe("yesterday");
    expect(agoWord("2026-10-30T10:00:00Z", NOW)).toBe("5 days ago");
    expect(agoWord("garbage", NOW)).toBe("");
  });
});

describe("updateWord", () => {
  it("says nothing was checked, up to date, newer, or why it failed", () => {
    expect(updateWord(undefined, NOW)).toEqual({ text: "Not checked yet.", tone: "muted" });
    expect(updateWord(check(), NOW)).toEqual({ text: "Up to date (0.3.3), checked 3 min ago.", tone: "muted" });
    expect(updateWord(check({ latest: "0.1.0" }), NOW)).toEqual({ text: "You have 0.3.3, ahead of the newest release (0.1.0); checked 3 min ago.", tone: "muted" });
    expect(updateWord(check({ latest: "0.3.4", newer: true }), NOW)).toEqual({ text: "Stillroom 0.3.4 is out; you have 0.3.3.", tone: "green" });
    expect(updateWord(check({ latest: null, error: "could not reach GitHub (offline?)" }), NOW)).toEqual({ text: "Could not check 3 min ago: could not reach GitHub (offline?).", tone: "amber" });
  });
});
