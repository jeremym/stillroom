// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { InstalledModel, ModelsResponse, Settings } from "../types";

// Neither the engine nor the Tauri event bus is here; both land on mocks.
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("../api", () => ({
  api: {
    getSettings: vi.fn(() => Promise.resolve({} as Settings)),
    setSettings: vi.fn((patch: Settings) => Promise.resolve(patch)),
    listModels: vi.fn(() => Promise.resolve({ providers: [], models: [] } as ModelsResponse)),
    runtimeStatus: vi.fn(() => Promise.reject(new Error("not here"))),
    listRecipes: vi.fn(() => Promise.reject(new Error("not here"))),
    listWatches: vi.fn(() => Promise.resolve([])),
    hasSecret: vi.fn(() => Promise.resolve(false)),
    codexStatus: vi.fn(() => Promise.reject(new Error("not here"))),
    permissionStatus: vi.fn(() => Promise.resolve([])),
    listSystemPrompts: vi.fn(() => Promise.resolve([])),
    tryModel: vi.fn(),
    tryPlanning: vi.fn(() => Promise.resolve({ ms: 20_000, clean: false, problems: ["the strip named a stage twice"] })),
    stopRun: vi.fn(() => Promise.resolve(true)),
  },
}));
import { api } from "../api";
import SettingsApp from "./SettingsApp";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function model(name: string, capabilities: InstalledModel["capabilities"]): InstalledModel {
  return { id: `ollama:${name}`, provider: "ollama", name, capabilities, contextWindow: 32768, parameterSize: "8.2B", local: true };
}
const MODELS: ModelsResponse = {
  providers: [{ id: "ollama", label: "Ollama", baseUrl: "http://localhost:11434", available: true, local: true }],
  models: [model("qwen3:8b", ["text", "structured_output"]), model("llama3.2:3b", ["text"]), model("gemma3:12b", ["text", "structured_output", "vision"])],
};

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

async function mountModels() {
  await act(async () => { root.render(<SettingsApp />); });
  await act(async () => { (host.querySelector('nav button[aria-label="Models"]') as HTMLElement).click(); });
}
const rows = () => Array.from(host.querySelectorAll("table tbody tr"));
const rowOf = (name: string) => rows().find((r) => r.querySelector("td.mono")?.textContent === name)!;
const planButton = (row: Element) => Array.from(row.querySelectorAll(".try-planning button.link")).find((b) => b.textContent?.startsWith("Try planning"));

describe("Settings › Models", () => {
  it("offers Try planning only on models that answer in a fixed shape, with the planner note once above the table", async () => {
    vi.mocked(api.listModels).mockResolvedValue(MODELS);
    await mountModels();
    expect(host.querySelectorAll(".planner-note").length).toBe(1);
    expect(host.querySelector(".planner-note")?.textContent).toContain("Not every model plans well; the planner");
    expect(rows().length).toBe(3);
    expect(planButton(rowOf("qwen3:8b"))?.textContent).toBe("Try planning");
    expect(planButton(rowOf("gemma3:12b"))?.textContent).toBe("Try planning");
    expect(planButton(rowOf("llama3.2:3b"))).toBeUndefined();
    // Every row still has the plain Try.
    expect(rows().every((r) => Array.from(r.querySelectorAll("button")).some((b) => b.textContent === "Try"))).toBe(true);
  });
  it("wears what was measured as a tag: the time when clean, amber words when not", async () => {
    vi.mocked(api.listModels).mockResolvedValue(MODELS);
    vi.mocked(api.getSettings).mockResolvedValue({ planner: { measured: { "ollama:qwen3:8b": { ms: 48_000, clean: true, at: "2026-11-02T10:00:00Z" }, "ollama:gemma3:12b": { ms: 20_000, clean: false } } } });
    await mountModels();
    const q = rowOf("qwen3:8b").querySelector(".try-planning .tag")!;
    expect(q.textContent).toBe("plans in 48 s");
    expect(q.classList.contains("amber")).toBe(false);
    expect(planButton(rowOf("qwen3:8b"))?.textContent).toBe("Try planning again");
    const g = rowOf("gemma3:12b").querySelector(".try-planning .tag")!;
    expect(g.textContent).toBe("plan had problems");
    expect(g.classList.contains("amber")).toBe(true);
  });
  it("Try planning runs the trial for that model and shows the first problem when it was not clean", async () => {
    vi.mocked(api.listModels).mockResolvedValue(MODELS);
    await mountModels();
    await act(async () => { (planButton(rowOf("gemma3:12b")) as HTMLElement).click(); });
    expect(api.tryPlanning).toHaveBeenCalledWith("ollama:gemma3:12b");
    expect(rowOf("gemma3:12b").querySelector(".try-planning")?.textContent).toContain("the strip named a stage twice");
    expect(api.tryPlanning).toHaveBeenCalledTimes(1);
  });
});
