import { describe, expect, it } from "vitest";
import { pullBar } from "./pull";

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
