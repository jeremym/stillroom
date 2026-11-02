// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { RecipeInfo } from "../types";

// The engine is not here; the planner preview lands on this mock instead.
vi.mock("../api", () => ({
  api: {
    previewPlanner: vi.fn(() => Promise.resolve({ resolution: null, problem: null, memoryBytes: null, candidates: [] })),
    tryPlanning: vi.fn(() => Promise.resolve({ ms: 48_000, clean: true, problems: [] })),
    stopRun: vi.fn(() => Promise.resolve(true)),
    getPrompt: vi.fn(() => Promise.resolve(null)),
  },
}));
import { api } from "../api";
import { NewRecipe } from "./NewRecipe";
import type { PlannerCandidate, PlannerPreview } from "../types";

const GB = 1e9;
function candidate(extra: Partial<PlannerCandidate> = {}): PlannerCandidate {
  return { modelId: "ollama:qwen3:14b", name: "qwen3:14b", parameterSize: "14.8B", sizeBytes: 9.3 * GB, fits: true, rank: 1, measuredMs: 48_000, measuredClean: true, reason: "fits in memory, answers in a fixed shape (JSON), planned clean in 48 s", local: true, ...extra };
}
const PREVIEW: PlannerPreview = {
  resolution: { slot: "planner", modelId: "ollama:qwen3:14b", via: "capability", reason: "fits in memory, answers in a fixed shape (JSON), planned clean in 48 s" },
  problem: null,
  memoryBytes: 36 * 2 ** 30,
  candidates: [
    candidate(),
    candidate({ modelId: "ollama:glm-4.7-flash:q8", name: "glm-4.7-flash:q8", parameterSize: "29.9B", sizeBytes: 29.9 * GB, fits: false, rank: 2, measuredMs: null, measuredClean: null, reason: "larger than this Mac holds comfortably" }),
    candidate({ modelId: "codex:gpt-5", name: "gpt-5", parameterSize: null, sizeBytes: null, rank: 3, measuredMs: null, measuredClean: null, reason: "runs in the cloud", local: false }),
  ],
};

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function recipe(id: string, extra: Partial<RecipeInfo> = {}): RecipeInfo {
  return { id, name: `Recipe ${id}`, version: null, description: "A line about it.", source: "builtin", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [], ...extra };
}

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.clearAllMocks();
});

const click = (el: Element | null | undefined) => act(() => { (el as HTMLElement).click(); });
const tab = (name: string) => Array.from(host.querySelectorAll('[role="tab"]')).find((b) => b.textContent?.includes(name));
const labels = () => Array.from(host.querySelectorAll(".field label strong")).map((s) => s.textContent);
const hint = () => host.querySelector(".tabrow-hint")?.textContent;

async function mount(extra: Partial<Parameters<typeof NewRecipe>[0]> = {}) {
  await act(async () => {
    root.render(<NewRecipe recipes={[recipe("a"), recipe("b")]} busy={false} error={null} onCreate={() => {}} onCancel={() => {}} {...extra} />);
  });
}

describe("NewRecipe", () => {
  it("opens on A goal: tabs on the zone's edge, the goal's hint first, the goal fields in the panel", async () => {
    await mount();
    const zone = host.querySelector("section.zone")!;
    expect(zone.firstElementChild?.classList.contains("zone-tabs")).toBe(true);
    expect(host.querySelector('[role="tablist"]')?.getAttribute("aria-label")).toBe("Start from");
    expect(tab("A goal")?.getAttribute("aria-selected")).toBe("true");
    expect(hint()).toContain("The planner drafts the steps");
    expect(labels()).toEqual(["What should this recipe do? *", "Built into the recipe", "Plan with", "Name", "Icon"]);
    expect(host.querySelector("label strong.glow-amber")?.textContent).toContain("*");
    expect(host.querySelector(".material-note")?.textContent).toContain("travels with it when shared");
    expect(host.querySelector(".material-note")?.textContent).toContain("Asked for each run: an input");
    expect((host.querySelector("textarea") as HTMLTextAreaElement).placeholder).toMatch(/^Say in plain words what you want it to do/);
    expect((host.querySelectorAll("textarea")[1] as HTMLTextAreaElement).placeholder).toMatch(/^Only what never changes between runs/);
    expect(api.previewPlanner).toHaveBeenCalled();
    expect(host.querySelector(".choices, .choice")).toBeNull();
  });
  it("switching tabs changes the form and the hint", async () => {
    await mount();
    click(tab("Blank"));
    expect(tab("Blank")?.getAttribute("aria-selected")).toBe("true");
    expect(tab("A goal")?.getAttribute("aria-selected")).toBe("false");
    expect(hint()).toContain("Reads one file and hands it back");
    expect(labels()).toEqual(["Name", "Icon"]);
    click(tab("A copy of a recipe"));
    expect(hint()).toContain("Everything from an existing recipe");
    expect(labels()).toEqual(["Copy of", "Name", "Icon"]);
    const options = Array.from(host.querySelectorAll("select option")).map((o) => o.textContent);
    expect(options).toEqual(["Recipe a (built-in)", "Recipe b (built-in)"]);
  });
  it("lists the planners in Auto's order with size and where, and says Auto's reason and cost", async () => {
    vi.mocked(api.previewPlanner).mockResolvedValue(PREVIEW);
    await mount();
    const options = Array.from(host.querySelectorAll('select[aria-label="Planning model"] option')).map((o) => o.textContent);
    expect(options).toEqual(["Auto — qwen3:14b (Auto)", "qwen3:14b · 14.8B · local", "glm-4.7-flash:q8 · 29.9B · local", "gpt-5 · cloud"]);
    const planLabel = Array.from(host.querySelectorAll(".field label")).find((l) => l.textContent?.startsWith("Plan with"));
    expect(planLabel?.textContent).toContain("Auto picks a model that fits in memory, answers in a fixed shape (JSON), and has planned quickly before");
    const cost = host.querySelector('[data-testid="plan-cost"]')!;
    expect(cost.textContent).toContain("qwen3:14b · 9.3 GB · fits in memory, answers in a fixed shape (JSON), planned clean in 48 s");
    expect(cost.classList.contains("amber")).toBe(false);
    expect(cost.querySelector("button.link")?.textContent).toBe("measure again");
  });
  it("a picked model that does not fit gets the amber cost line, and measure it runs a trial plan then re-reads the preview", async () => {
    vi.mocked(api.previewPlanner).mockResolvedValue(PREVIEW);
    await mount();
    const select = host.querySelector('select[aria-label="Planning model"]') as HTMLSelectElement;
    await act(async () => {
      select.value = "ollama:glm-4.7-flash:q8";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    const cost = host.querySelector('[data-testid="plan-cost"]')!;
    expect(cost.classList.contains("amber")).toBe(true);
    expect(cost.textContent).toContain("29.9 GB on a 36 GB Mac: larger than this Mac holds comfortably; a smaller model plans in a fraction of the time · not yet measured");
    const calls = vi.mocked(api.previewPlanner).mock.calls.length;
    await act(async () => { (cost.querySelector("button.link") as HTMLElement).click(); });
    expect(api.tryPlanning).toHaveBeenCalledWith("ollama:glm-4.7-flash:q8");
    expect(vi.mocked(api.previewPlanner).mock.calls.length).toBe(calls + 1);
  });
  it("renders what is passed below under the Draft it row", async () => {
    await mount({ below: <div data-testid="progress">drafting…</div> });
    const row = Array.from(host.querySelectorAll(".row")).find((r) => r.textContent?.includes("Draft it"))!;
    expect(row.nextElementSibling?.getAttribute("data-testid")).toBe("progress");
  });
});
