// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { InstalledModel, ModelsResponse, PlannerPreview, RecipeInfo } from "../types";

// The engine is not here; the planner preview and the model list land on mocks instead.
vi.mock("../api", () => ({
  api: {
    previewPlanner: vi.fn(() => Promise.resolve({ resolution: null, problem: null, memoryBytes: null, candidates: [] })),
    tryPlanning: vi.fn(() => Promise.resolve({ ms: 48_000, clean: true, problems: [] })),
    stopRun: vi.fn(() => Promise.resolve(true)),
    getPrompt: vi.fn(() => Promise.resolve(null)),
    pullModel: vi.fn(() => Promise.resolve()),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
import { api } from "../api";
import { FirstRun } from "./FirstRun";
import { EXAMPLE_GOALS } from "../firstRun/examples";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const GB = 1e9;
const PREVIEW: PlannerPreview = {
  resolution: { slot: "planner", modelId: "ollama:qwen3:14b", via: "capability", reason: "fits in memory, answers in a fixed shape (JSON), planned clean in 48 s" },
  problem: null,
  memoryBytes: 36 * 2 ** 30,
  candidates: [
    { modelId: "ollama:qwen3:14b", name: "qwen3:14b", parameterSize: "14.8B", sizeBytes: 9.3 * GB, fits: true, rank: 1, measuredMs: 48_000, measuredClean: true, reason: "fits in memory", local: true },
    { modelId: "ollama:glm-4.7-flash:q8", name: "glm-4.7-flash:q8", parameterSize: "29.9B", sizeBytes: 29.9 * GB, fits: false, rank: 2, measuredMs: null, measuredClean: null, reason: "larger than this Mac holds comfortably", local: true },
  ],
};
function model(name: string, extra: Partial<InstalledModel> = {}): InstalledModel {
  return { id: `ollama:${name}`, provider: "ollama", name, capabilities: ["text", "structured_output"], contextWindow: null, parameterSize: null, local: true, ...extra };
}
function models(list: InstalledModel[], available = true): ModelsResponse {
  return { providers: [{ id: "ollama", label: "Ollama", baseUrl: "http://localhost:11434", available, local: true }], models: list };
}
const FOUR = models([model("qwen3:14b"), model("glm-4.7-flash:q8"), model("llama3.2"), model("nomic-embed-text", { capabilities: ["embedding"] })]);
function recipe(id: string): RecipeInfo {
  return { id, name: `Recipe ${id}`, version: null, description: "A line about it.", source: "builtin", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [] };
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
const textareas = () => Array.from(host.querySelectorAll("textarea")) as HTMLTextAreaElement[];
const button = (text: string) => Array.from(host.querySelectorAll("button")).find((b) => b.textContent?.trim() === text);
/** Type into a controlled field the way a person does: through the element's own setter, so React sees the change. */
const type = (el: HTMLInputElement | HTMLTextAreaElement, value: string) => act(async () => {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
});
const draft = () => Array.from(host.querySelectorAll("button.primary")).find((b) => b.textContent?.includes("Draft it")) as HTMLButtonElement | undefined;

type P = Parameters<typeof FirstRun>[0];
async function mount(extra: Partial<P> = {}) {
  const props: P = {
    recipes: [recipe("summarize-file")], busy: false, error: null, onCreate: () => {}, models: FOUR,
    onRefreshModels: () => {}, onOpenSettings: () => {}, onSave: () => {}, onBrowse: () => {}, onSkip: () => {}, ...extra,
  };
  await act(async () => { root.render(<FirstRun {...props} />); });
}

describe("FirstRun", () => {
  it("is the goal screen alone: no tab row, the five examples under the goal box, no Plan with field, the cannot-yet fold", async () => {
    await mount();
    expect(host.querySelector('[role="tablist"]')).toBeNull();
    expect(host.querySelector(".boot-line")?.textContent).toBe("Describe the goal. Get the result. See how it was made.");
    const items = Array.from(host.querySelectorAll(".examples li button")).map((b) => b.textContent);
    expect(items).toEqual(EXAMPLE_GOALS.map((e) => e.sentence));
    expect(host.querySelector(".examples p")?.textContent).toBe("Or start from one of these:");
    const labels = Array.from(host.querySelectorAll(".field label strong")).map((s) => s.textContent);
    expect(labels).toEqual(["What should this recipe do? *", "Built into the recipe", "Name", "Icon"]);
    const fold = host.querySelector("details.cannot-yet")!;
    expect(fold.querySelector("summary")?.textContent).toContain("What Stillroom cannot do yet");
    expect(fold.querySelector("p")?.textContent).toContain("Run on a schedule");
    expect(button("Cancel")).toBeUndefined();
  });
  it("clicking an example fills the goal box (and clears the material box) and puts the cursor there", async () => {
    await mount();
    const [goal, material] = textareas();
    await type(material, "stale");
    expect(material.value).toBe("stale");
    click(Array.from(host.querySelectorAll(".examples li button"))[2]);
    expect(goal.value).toBe(EXAMPLE_GOALS[2].sentence);
    expect(material.value).toBe("");
    expect(document.activeElement).toBe(goal);
  });
  it("the plumbing line reads the preview: provider, model count, who will plan and the fit; change unfolds the picker", async () => {
    vi.mocked(api.previewPlanner).mockResolvedValue(PREVIEW);
    await mount();
    const line = host.querySelector(".plumbing-line")!;
    expect(host.querySelector(".plumbing")?.getAttribute("data-state")).toBe("ready");
    expect(line.querySelector(".pill")?.textContent).toBe("Ollama · 4 models");
    expect(line.querySelector(".pill .dot")?.classList.contains("ok")).toBe(true);
    expect(line.textContent).toContain("qwen3:14b will plan (fits in memory, plans in 48 s)");
    expect(host.querySelector('select[aria-label="Planning model"]')).toBeNull();
    click(button("change"));
    expect(host.querySelector('select[aria-label="Planning model"]')).not.toBeNull();
    expect(host.querySelector(".plumbing-help")).toBeNull();
  });
  it("with Ollama down the line says so, the install help unfolds, and Draft it is held with the reason", async () => {
    await mount({ models: models([], false) });
    expect(host.querySelector(".plumbing")?.getAttribute("data-state")).toBe("down");
    expect(host.querySelector(".plumbing-line .pill")?.textContent).toBe("Ollama · not running");
    expect(host.querySelector('[data-testid="ollama-down"]')?.textContent).toContain("Ollama is not answering at http://localhost:11434");
    expect(draft()?.disabled).toBe(true);
    expect(draft()?.textContent).toContain("needs a model that can plan");
  });
  it("with models but none that can plan, the starter pull unfolds and Draft it is held", async () => {
    vi.mocked(api.previewPlanner).mockResolvedValue({ resolution: null, problem: "no model can plan", memoryBytes: 16 * 2 ** 30, candidates: [] });
    await mount({ models: models([model("llama3.2", { capabilities: ["text"] })]) });
    expect(host.querySelector(".plumbing")?.getAttribute("data-state")).toBe("no-planner");
    expect(host.querySelector(".plumbing-line")?.textContent).toContain("none can plan");
    expect(host.querySelector('[data-testid="needs-planner"]')?.textContent).toContain("Pull qwen3:8b");
    expect(draft()?.disabled).toBe(true);
    expect(draft()?.textContent).toContain("needs a model that can plan");
  });
  it("skip and browse mark the first run done and call their handlers", async () => {
    const onSave = vi.fn(); const onBrowse = vi.fn(); const onSkip = vi.fn();
    await mount({ onSave, onBrowse, onSkip });
    click(button("browse the catalog"));
    expect(onSave).toHaveBeenCalledWith({ firstRun: false });
    expect(onBrowse).toHaveBeenCalledTimes(1);
    expect(onSkip).not.toHaveBeenCalled();
    click(button("skip for now"));
    expect(onSave).toHaveBeenCalledTimes(2);
    expect(onSkip).toHaveBeenCalledTimes(1);
  });
  it("Draft it hands the goal to onCreate as a goal request once a name is typed", async () => {
    vi.mocked(api.previewPlanner).mockResolvedValue(PREVIEW);
    const onCreate = vi.fn();
    await mount({ onCreate });
    click(Array.from(host.querySelectorAll(".examples li button"))[0]);
    const name = host.querySelector('input[type="text"]') as HTMLInputElement;
    await type(name, "Morning brief");
    expect(draft()?.disabled).toBe(false);
    click(draft());
    expect(onCreate).toHaveBeenCalledTimes(1);
    expect(onCreate.mock.calls[0][0]).toMatchObject({ mode: "goal", id: "morning-brief", name: "Morning brief", goal: EXAMPLE_GOALS[0].sentence, model: null });
  });
});
