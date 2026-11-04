// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { EDIT_WORDS, PlanNote, PlanStreamView, planHeader } from "./PlanStreamView";
import { EMPTY_STREAM, type PlanStreamState } from "../planStream";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const now = 1_700_000_000_000;
let host: HTMLDivElement;
let root: Root;
beforeEach(() => { vi.useFakeTimers({ now, toFake: ["Date", "setInterval", "clearInterval"] }); host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.useRealTimers(); });

const strip: PlanStreamState = { ...EMPTY_STREAM, phase: "strip", attempt: 1, of: 2, elapsedMs: 48_000, tokens: 612, at: now, text: '{\n  "stages": [', lines: ["{", '  "stages": ['] };

describe("PlanStreamView", () => {
  it("reads the header as the plan says, and announces only the header", () => {
    act(() => root.render(<PlanStreamView state={strip} />));
    const head = host.querySelector(".plan-stream-head")!;
    expect(head.textContent).toBe("Choosing the stages · call 1 of 2 · 48 s · 612 tokens");
    expect(head.getAttribute("aria-live")).toBe("polite");
    const text = host.querySelector(".plan-stream-text")!;
    expect(text.getAttribute("aria-live")).toBeNull();
    expect(text.textContent).toContain('"stages": [');
    expect(host.querySelector(".cursor")).not.toBeNull();
    expect(host.querySelector("textarea, [contenteditable]")).toBeNull();
    // The clock ticks on its own between events.
    act(() => { vi.advanceTimersByTime(3_000); });
    expect(head.textContent).toBe("Choosing the stages · call 1 of 2 · 51 s · 612 tokens");
  });
  it("adds the time since the last event, and stops the clock and the caret when done", () => {
    expect(planHeader(strip, now + 5_400)).toBe("Choosing the stages · call 1 of 2 · 53 s · 612 tokens");
    expect(planHeader({ ...strip, done: true }, now + 5_400)).toBe("Choosing the stages · call 1 of 2 · 48 s · 612 tokens");
    // An edit (Ask for a change) says what it is doing in its own words.
    const edit = { ...strip, phase: "steps" as const, attempt: 1, of: 3, modelPhase: "reading" as const };
    expect(planHeader(edit, now + 5_400, EDIT_WORDS)).toBe("Rewriting the recipe · call 1 of 3 · reading the recipe · 53 s · 612 tokens");
    act(() => root.render(<PlanStreamView state={{ ...strip, done: true }} />));
    expect(host.querySelector(".cursor")).toBeNull();
  });
  it("says a fix round with its problems and try count", () => {
    expect(planHeader({ ...strip, phase: "fix", attempt: 2, of: 3, problems: ["a", "b"], elapsedMs: 130_000, done: true }, now))
      .toBe("Fixing 2 problems · try 2 of 3 · 2 min 10 s · 612 tokens");
  });
  it("shows nothing before the first event", () => {
    act(() => root.render(<PlanStreamView state={EMPTY_STREAM} />));
    expect(host.querySelector(".plan-stream")).toBeNull();
  });
});

describe("PlanNote", () => {
  it("keeps the transcript behind a quiet toggle", () => {
    act(() => root.render(<PlanNote note="Drafted by q (local) on the first try in 48 s." transcript={"── call 1 (strip) ──\n{}"} />));
    expect(host.textContent).toContain("Drafted by q");
    expect(host.querySelector(".plan-stream")).toBeNull();
    const btn = host.querySelector("button")!;
    expect(btn.textContent).toBe("show what it wrote");
    act(() => btn.click());
    expect(host.querySelector(".plan-stream-text")!.textContent).toContain("── call 1 (strip) ──");
    expect(host.querySelector("button")!.textContent).toBe("hide what it wrote");
  });
  it("has no toggle without a transcript", () => {
    act(() => root.render(<PlanNote note="Drafted." transcript={null} />));
    expect(host.querySelector("button")).toBeNull();
  });
});
