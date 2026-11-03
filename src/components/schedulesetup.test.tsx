// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { Grant, RecipeInfo } from "../types";

// The engine is not here; Set up's calls land on this mock instead.
vi.mock("../api", () => ({
  api: {
    estimate: vi.fn(() => Promise.resolve(null)),
    previewResolution: vi.fn(() => Promise.resolve({ resolutions: {}, problems: {} })),
    grantPermissions: vi.fn(() => Promise.resolve({})),
    revokePermissions: vi.fn(() => Promise.resolve({})),
  },
  fmtTokens: (n: number) => String(n),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
import { RecipePanel } from "./RecipePanel";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function recipe(extra: Partial<RecipeInfo> = {}): RecipeInfo {
  return { id: "brief", name: "Morning brief", version: "1.0.0", description: null, source: "user", shelf: "shelf", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [{ id: "make", name: "Make" }], ...extra };
}
const noop = () => {};
type OnSchedule = NonNullable<Parameters<typeof RecipePanel>[0]["onSchedule"]>;
const accepts = () => vi.fn<OnSchedule>(() => Promise.resolve(true));

let host: HTMLDivElement;
let root: Root;
beforeEach(() => { host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.clearAllMocks(); });

const buttons = () => Array.from(host.querySelectorAll("button"));
const button = (text: string) => buttons().find((b) => b.textContent?.trim() === text || b.textContent?.trim().startsWith(text));
const settle = () => act(async () => { await Promise.resolve(); });
const click = (b: HTMLButtonElement | undefined) => act(() => { b!.click(); });

function render(r: RecipeInfo, onSchedule = accepts(), onSeeSchedules = noop) {
  act(() => root.render(
    <RecipePanel recipes={[r]} recipe={r} onSelect={noop} pins={{ writer: "ollama:qwen3:8b" }} onPins={noop} models={[]} running={false}
      onRun={noop} onWatch={noop} onSchedule={onSchedule} onSeeSchedules={onSeeSchedules} />,
  ));
  return onSchedule;
}

describe("Run every day at… in Set up", () => {
  it("opens a small form with the time and every day on, and adds the schedule with Set up's inputs and pins", async () => {
    const onSchedule = render(recipe({ inputs: { topic: { type: "string", label: "Topic", default: "weather" } }, stages: [{ id: "make", name: "Make", inputs: ["topic"] }] }));
    await settle();
    const opener = button("Run every day at…") as HTMLButtonElement;
    expect(opener.disabled).toBe(false);
    expect(host.querySelector(".schedule-form")).toBeNull();
    click(opener);
    const form = host.querySelector(".schedule-form")!;
    expect(form).toBeTruthy();
    const time = form.querySelector<HTMLInputElement>("input[type=time]")!;
    expect(time.value).toBe("07:30");
    const days = Array.from(form.querySelectorAll<HTMLButtonElement>(".days button"));
    expect(days.map((d) => d.textContent)).toEqual(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);
    expect(days.every((d) => d.getAttribute("aria-pressed") === "true")).toBe(true);
    expect(form.textContent).toContain("Runs every day at 07:30");

    click(button("Add the schedule") as HTMLButtonElement);
    await settle();
    expect(onSchedule).toHaveBeenCalledTimes(1);
    const [r, inputs, pins, when, picked] = onSchedule.mock.calls[0];
    expect(r.id).toBe("brief");
    expect(inputs).toEqual({ topic: "weather" });
    expect(pins).toEqual({ writer: "ollama:qwen3:8b" });
    expect(when).toBe("07:30");
    expect(picked).toEqual([]);
    await settle();
    expect(host.querySelector(".schedule-form")).toBeNull();
    expect(host.textContent).toContain("Runs every day at 07:30 while Stillroom is open");
    expect(button("see schedules")).toBeTruthy();
  });

  it("a changed time and days off go to the engine in week order", async () => {
    const onSchedule = render(recipe());
    await settle();
    click(button("Run every day at…") as HTMLButtonElement);
    const form = host.querySelector(".schedule-form")!;
    const time = form.querySelector<HTMLInputElement>("input[type=time]")!;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(time, "18:00");
      time.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const day = (label: string) => Array.from(form.querySelectorAll<HTMLButtonElement>(".days button")).find((b) => b.textContent === label)!;
    click(day("Sat"));
    click(day("Sun"));
    expect(day("Sat").getAttribute("aria-pressed")).toBe("false");
    expect(form.textContent).toContain("Runs weekdays at 18:00");
    click(button("Add the schedule") as HTMLButtonElement);
    await settle();
    expect(onSchedule.mock.calls[0]?.[3]).toBe("18:00");
    expect(onSchedule.mock.calls[0]?.[4]).toEqual(["mon", "tue", "wed", "thu", "fri"]);
  });

  it("with every day off, Add waits; with the engine refusing, the form stays and no line shows", async () => {
    const onSchedule = render(recipe(), vi.fn<OnSchedule>(() => Promise.resolve(false)));
    await settle();
    click(button("Run every day at…") as HTMLButtonElement);
    const form = host.querySelector(".schedule-form")!;
    form.querySelectorAll<HTMLButtonElement>(".days button").forEach((b) => click(b));
    expect((button("Add the schedule") as HTMLButtonElement).disabled).toBe(true);
    click(Array.from(form.querySelectorAll<HTMLButtonElement>(".days button"))[0]);
    click(button("Add the schedule") as HTMLButtonElement);
    await settle();
    expect(onSchedule).toHaveBeenCalled();
    expect(host.querySelector(".schedule-form")).toBeTruthy();
    expect(host.textContent).not.toContain("while Stillroom is open · ");
  });

  it("is held the same way Run is: by the consent step and by a missing input", async () => {
    const home: Grant = { kind: "read", entry: "~", text: "Read every file in your home folder." };
    render(recipe({ consent: { needed: [home], granted: [] } }));
    await settle();
    expect((button("Run every day at…") as HTMLButtonElement).disabled).toBe(true);

    render(recipe({ inputs: { folder: { type: "directory", label: "Folder" } }, stages: [{ id: "make", name: "Make", inputs: ["folder"] }] }));
    await settle();
    expect((button("Run") as HTMLButtonElement).textContent).toContain("needs Folder");
    expect((button("Run every day at…") as HTMLButtonElement).disabled).toBe(true);
    expect(button("Run every day at…")).not.toBe(button("Run"));
  });
});
