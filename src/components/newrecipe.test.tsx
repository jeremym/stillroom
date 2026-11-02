// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { RecipeInfo } from "../types";

// The engine is not here; the planner preview lands on this mock instead.
vi.mock("../api", () => ({
  api: {
    previewPlanner: vi.fn(() => Promise.resolve({ resolution: null, problem: null, candidates: [] })),
    getPrompt: vi.fn(() => Promise.resolve(null)),
  },
}));
import { api } from "../api";
import { NewRecipe } from "./NewRecipe";

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
    expect(labels()).toEqual(["What should this recipe do?", "Built into the recipe", "Plan with", "Name", "Icon"]);
    expect(host.querySelector(".material-note")?.textContent).toContain("travels with it when shared");
    expect(host.querySelector(".material-note")?.textContent).toContain("ask each run instead");
    expect((host.querySelector("textarea") as HTMLTextAreaElement).placeholder).toMatch(/^Ask me for a folder of PDFs/);
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
  it("renders what is passed below under the Draft it row", async () => {
    await mount({ below: <div data-testid="progress">drafting…</div> });
    const row = Array.from(host.querySelectorAll(".row")).find((r) => r.textContent?.includes("Draft it"))!;
    expect(row.nextElementSibling?.getAttribute("data-testid")).toBe("progress");
  });
});
