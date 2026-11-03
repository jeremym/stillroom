import { describe, expect, it } from "vitest";
import { CANNOT_YET, EXAMPLE_GOALS } from "./examples";

describe("example goals", () => {
  it("are five, with distinct ids and sentences, each naming the catalog recipe it resembles", () => {
    expect(EXAMPLE_GOALS).toHaveLength(5);
    expect(new Set(EXAMPLE_GOALS.map((e) => e.id)).size).toBe(5);
    expect(new Set(EXAMPLE_GOALS.map((e) => e.sentence)).size).toBe(5);
    for (const e of EXAMPLE_GOALS) {
      expect(e.id).toMatch(/^[a-z][a-z0-9-]*$/);
      expect(e.sentence.endsWith(".")).toBe(true);
      expect(e.like).toMatch(/^[a-z][a-z0-9-]*$/);
    }
  });
  it("promise only what the engine does: no sending, no services; a schedule is fine now that schedules exist", () => {
    const all = EXAMPLE_GOALS.map((e) => e.sentence).join(" ");
    expect(all).not.toMatch(/Notion|Slack|send it|post it/i);
    expect(EXAMPLE_GOALS.find((e) => e.id === "news-brief")?.sentence).toMatch(/^Each morning/);
    expect(EXAMPLE_GOALS.find((e) => e.id === "mail-sorter")?.sentence).toContain("for me to send");
  });
  it("the cannot-yet line names the five gaps", () => {
    for (const gap of ["Send mail", "Notion, Slack", "Read a mailbox", "Transcribe a recording", "Run on a schedule"]) expect(CANNOT_YET).toContain(gap);
  });
});
