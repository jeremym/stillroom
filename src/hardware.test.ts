import { describe, expect, it } from "vitest";
import { fitOf, fitWord, fmtSpeed, speedOf, thisMacLine } from "./hardware";

const GB = 2 ** 30;

describe("This Mac", () => {
  it("says every fact that was read, in order, and nothing for one that was not", () => {
    expect(thisMacLine({ chip: "Apple M3 Max", memoryBytes: 96 * GB, gpuCores: 40, cpuCores: 16, modelCeilingBytes: 57.6 * GB })).toBe(
      "Apple M3 Max · 96 GB unified memory (shared by the CPU and GPU) · 40 GPU cores · 16 CPU cores · room for models up to about 58 GB",
    );
    expect(thisMacLine({ chip: null, memoryBytes: 36 * GB, gpuCores: null, cpuCores: null, modelCeilingBytes: 21.6 * GB })).toBe(
      "36 GB unified memory (shared by the CPU and GPU) · room for models up to about 22 GB",
    );
    expect(thisMacLine({ chip: null, memoryBytes: null, gpuCores: null, cpuCores: null, modelCeilingBytes: null })).toBeNull();
    expect(thisMacLine(null)).toBeNull();
  });
  it("says how a model sits in memory: room under 40 %, tight to 60 %, too large past that", () => {
    expect(fitOf(10 * GB, 36 * GB)).toBe("room");
    expect(fitOf(20 * GB, 36 * GB)).toBe("tight");
    expect(fitOf(30 * GB, 36 * GB)).toBe("too_large");
    expect(fitOf(null, 36 * GB)).toBe("unknown");
    expect(fitOf(10 * GB, null)).toBe("unknown");
    expect(["room", "tight", "too_large", "unknown"].map((f) => fitWord(f as never))).toEqual(["fits with room", "fits, but tightly", "too large for this Mac", "size not known"]);
  });
  it("words a speed, or a dash before the model has answered", () => {
    expect(fmtSpeed(66.4)).toBe("66 tok/s");
    expect(fmtSpeed(4.26)).toBe("4.3 tok/s");
    expect(fmtSpeed(null)).toBe("—");
    expect(fmtSpeed(0)).toBe("—");
    expect(speedOf({ "ollama:qwen3:8b": { tokensPerSecond: 80, at: "2026-11-02T10:00:00Z" } }, "ollama:qwen3:8b")).toBe("80 tok/s");
    expect(speedOf({}, "ollama:qwen3:8b")).toBe("—");
    expect(speedOf(undefined, "ollama:qwen3:8b")).toBe("—");
  });
});
