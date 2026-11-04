import { describe, expect, it } from "vitest";
import { canPlan, costLine, fmtPlanTime, measuredTag, optionLabel } from "./planWith";
import type { PlannerCandidate } from "./types";

const GB = 1e9;
const MAC_36 = 36 * 2 ** 30;

function candidate(extra: Partial<PlannerCandidate> = {}): PlannerCandidate {
  return {
    modelId: "ollama:glm-4.7-flash:q8", name: "glm-4.7-flash:q8", parameterSize: "29.9B", sizeBytes: 29.9 * GB,
    fits: true, rank: 1, measuredMs: null, measuredClean: null, reason: "fits in memory", local: true, ...extra,
  };
}

describe("costLine", () => {
  it("fits and measured clean: size, fits in memory, the last trial's time", () => {
    const c = costLine(candidate({ sizeBytes: 9.3 * GB, measuredMs: 48_000, measuredClean: true }), MAC_36);
    expect(c.text).toBe("9.3 GB · fits in memory · the last trial plan took 48 s and checked clean");
    expect(c.amber).toBe(false);
  });
  it("says room or tight when the engine gives the finer verdict", () => {
    expect(costLine(candidate({ sizeBytes: 9.3 * GB, fit: "room" }), MAC_36).fit).toBe("fits with room");
    expect(costLine(candidate({ sizeBytes: 20 * GB, fit: "tight" }), MAC_36).fit).toBe("fits, but tightly");
  });
  it("does not fit: amber, the size against this Mac's memory, and the way out", () => {
    const c = costLine(candidate({ fits: false }), MAC_36);
    expect(c.text).toBe("29.9 GB on a 36 GB Mac: larger than this Mac holds comfortably; a smaller model plans in a fraction of the time · not yet measured");
    expect(c.amber).toBe(true);
    expect(c.size).toBe("29.9 GB");
  });
  it("measured with problems says so, past a minute in minutes", () => {
    const c = costLine(candidate({ measuredMs: 130_000, measuredClean: false }), MAC_36);
    expect(c.measured).toBe("the last trial had problems after 2 min 10 s");
    expect(c.amber).toBe(false);
  });
  it("not measured says so", () => {
    expect(costLine(candidate(), MAC_36).measured).toBe("not yet measured");
  });
  it("does not fit on a Mac whose memory is unknown still warns, without a memory figure", () => {
    expect(costLine(candidate({ fits: false }), null).fit).toBe("29.9 GB: larger than this Mac holds comfortably; a smaller model plans in a fraction of the time");
  });
  it("a cloud model is amber because what you type leaves this Mac, and has no size", () => {
    const c = costLine(candidate({ modelId: "codex:gpt-5", name: "gpt-5", parameterSize: null, sizeBytes: null, local: false }), MAC_36);
    expect(c.amber).toBe(true);
    expect(c.size).toBeNull();
    expect(c.text).toBe("runs in the cloud: what you type leaves this Mac · not yet measured");
  });
});

describe("optionLabel", () => {
  it("is name · size · where", () => {
    expect(optionLabel(candidate())).toBe("glm-4.7-flash:q8 · 29.9B · local");
    expect(optionLabel(candidate({ name: "gpt-5", parameterSize: null, local: false }))).toBe("gpt-5 · cloud");
  });
});

describe("measuredTag and fmtPlanTime", () => {
  it("wears the time when clean and amber words when not", () => {
    expect(measuredTag({ ms: 48_000, clean: true })).toEqual({ text: "plans in 48 s", amber: false });
    expect(measuredTag({ ms: 20_000, clean: false })).toEqual({ text: "plan had problems", amber: true });
    expect(measuredTag(undefined)).toBeNull();
  });
  it("rounds to seconds and turns to minutes at sixty", () => {
    expect(fmtPlanTime(480)).toBe("0 s");
    expect(fmtPlanTime(59_600)).toBe("1 min 00 s");
    expect(fmtPlanTime(130_400)).toBe("2 min 10 s");
  });
});

describe("canPlan", () => {
  it("needs text and structured output on a local model", () => {
    expect(canPlan({ capabilities: ["text", "structured_output"], local: true })).toBe(true);
    expect(canPlan({ capabilities: ["text"], local: true })).toBe(false);
    expect(canPlan({ capabilities: ["text", "structured_output"], local: false })).toBe(false);
  });
});
