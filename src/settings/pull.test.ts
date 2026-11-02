import { describe, expect, it } from "vitest";
import { pullBar, suggestPlanner } from "./pull";

describe("pullBar", () => {
  it("draws blocks for the share done", () => {
    const p = pullBar(500e6, 1000e6, 10);
    expect(p.bar).toBe("▓▓▓▓▓░░░░░");
    expect(p.pct).toBe(50);
    expect(p.text).toBe("500 MB of 1.0 GB");
  });
  it("is empty without a total", () => {
    const p = pullBar(null, null, 4);
    expect(p.bar).toBe("░░░░");
    expect(p.pct).toBeNull();
  });
  it("uses gigabytes past a billion", () => {
    expect(pullBar(2.5e9, 5e9, 4).text).toBe("2.5 GB of 5.0 GB");
  });
});

describe("suggestPlanner", () => {
  const GiB = 2 ** 30;
  it("names an 8 B planner on a 16 GB Mac and a 14 B one from 32 GB up; never a 30 B+ model", () => {
    expect(suggestPlanner(16 * GiB)).toBe("qwen3:8b");
    expect(suggestPlanner(32 * GiB)).toBe("qwen3:14b");
    expect(suggestPlanner(128 * GiB)).toBe("qwen3:14b");
  });
  it("plays safe when memory is unknown", () => {
    expect(suggestPlanner(null)).toBe("qwen3:8b");
  });
});
