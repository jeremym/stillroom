// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { Schedule } from "../types";

vi.mock("../api", () => ({
  api: {
    setScheduleEnabled: vi.fn(() => Promise.resolve({})),
    deleteSchedule: vi.fn(() => Promise.resolve()),
    fireSchedule: vi.fn(() => Promise.resolve({})),
  },
}));
import { api } from "../api";
import { SchedulePanel } from "./SchedulePanel";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const now = new Date(2026, 10, 2, 9, 0);
function schedule(extra: Partial<Schedule> = {}): Schedule {
  return {
    id: "s1", recipeId: "brief", recipeName: "Morning brief", inputs: {}, pins: {}, time: "07:30", days: [], enabled: true,
    createdAt: "2026-11-01T10:00:00Z", lastFiredAt: null, lastRunId: null, nextAt: new Date(2026, 10, 3, 7, 30).toISOString(), ...extra,
  };
}
const noop = () => {};

let host: HTMLDivElement;
let root: Root;
beforeEach(() => { host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.clearAllMocks(); });

const buttons = () => Array.from(host.querySelectorAll("button"));
const button = (text: string) => buttons().find((b) => b.textContent?.trim().includes(text));
const settle = () => act(async () => { await Promise.resolve(); });

function render(schedules: Schedule[], extra: { watchCount?: number; onChanged?: () => void; onOpenRun?: (id: string) => void } = {}) {
  act(() => root.render(
    <SchedulePanel schedules={schedules} watchCount={extra.watchCount ?? 0} onChanged={extra.onChanged ?? noop} onOpenRun={extra.onOpenRun ?? noop} onError={noop} now={() => now} />,
  ));
}

describe("the Schedules panel", () => {
  it("hides with nothing to show, and says schedules exist when there is a watch", () => {
    render([]);
    expect(host.textContent).toBe("");
    render([], { watchCount: 1 });
    expect(host.textContent).toContain("Schedules");
    expect(host.textContent).toContain("No schedules yet");
  });

  it("one row per schedule: name, when, next run, last run", () => {
    const open = vi.fn();
    render([
      schedule(),
      schedule({ id: "s2", recipeName: "Weekly digest", time: "18:00", days: ["mon", "wed", "fri"], lastFiredAt: "2026-11-01T18:00:00Z", lastRunId: "r9", nextAt: new Date(2026, 10, 2, 11, 0).toISOString() }),
    ], { onOpenRun: open });
    const rows = Array.from(host.querySelectorAll("li"));
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain("Morning brief");
    expect(rows[0].textContent).toContain("every day at 07:30");
    expect(rows[0].textContent).toContain("next tomorrow at 07:30");
    expect(rows[0].textContent).toContain("has not run yet");
    expect(rows[1].textContent).toContain("Mon, Wed, Fri at 18:00");
    expect(rows[1].textContent).toContain("next in 2 h");
    expect(rows[1].textContent).toContain("last ran");
    act(() => (rows[1].querySelector("button.link") as HTMLButtonElement).click());
    expect(open).toHaveBeenCalledWith("r9");
  });

  it("the toggle turns a schedule off and on, and the list is read again", async () => {
    const changed = vi.fn();
    render([schedule()], { onChanged: changed });
    const toggle = host.querySelector<HTMLButtonElement>("button.toggle")!;
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    act(() => toggle.click());
    expect(api.setScheduleEnabled).toHaveBeenCalledWith("s1", false);
    await settle();
    expect(changed).toHaveBeenCalled();

    render([schedule({ enabled: false, nextAt: null })]);
    expect(host.textContent).toContain("off");
    expect(host.textContent).not.toContain("next ");
    act(() => host.querySelector<HTMLButtonElement>("button.toggle")!.click());
    expect(api.setScheduleEnabled).toHaveBeenCalledWith("s1", true);
  });

  it("Run now fires the schedule", async () => {
    render([schedule()]);
    act(() => button("Run now")!.click());
    expect(api.fireSchedule).toHaveBeenCalledWith("s1");
    await settle();
  });

  it("Delete asks once, then deletes", async () => {
    const changed = vi.fn();
    render([schedule()], { onChanged: changed });
    act(() => button("Delete…")!.click());
    expect(api.deleteSchedule).not.toHaveBeenCalled();
    expect(button("Really delete")).toBeTruthy();
    act(() => button("Keep it")!.click());
    expect(button("Really delete")).toBeUndefined();
    act(() => button("Delete…")!.click());
    act(() => button("Really delete")!.click());
    expect(api.deleteSchedule).toHaveBeenCalledWith("s1");
    await settle();
    expect(changed).toHaveBeenCalled();
  });
});
