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
    hardware: vi.fn(() => Promise.resolve({ chip: null, memoryBytes: null, gpuCores: null, cpuCores: null, modelCeilingBytes: null })),
    revealPlansFolder: vi.fn(() => Promise.resolve()),
    runtimeStatus: vi.fn(() => Promise.reject(new Error("not here"))),
    listRecipes: vi.fn(() => Promise.reject(new Error("not here"))),
    listWatches: vi.fn(() => Promise.resolve([])),
    hasSecret: vi.fn(() => Promise.resolve(false)),
    codexStatus: vi.fn(() => Promise.reject(new Error("not here"))),
    permissionStatus: vi.fn(() => Promise.resolve([])),
    listSystemPrompts: vi.fn(() => Promise.resolve([])),
    readSystemPrompt: vi.fn(() => Promise.reject(new Error("not here"))),
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
  it("says what this Mac is made of above the table, and the room a model has", async () => {
    vi.mocked(api.listModels).mockResolvedValue(MODELS);
    vi.mocked(api.hardware).mockResolvedValue({ chip: "Apple M3 Max", memoryBytes: 96 * 2 ** 30, gpuCores: 40, cpuCores: 16, modelCeilingBytes: 57.6 * 2 ** 30 });
    await mountModels();
    const line = host.querySelector(".this-mac")!;
    expect(line.textContent).toBe("This Mac · Apple M3 Max · 96 GB unified memory (shared by the CPU and GPU) · 40 GPU cores · 16 CPU cores · room for models up to about 58 GB");
    expect(host.querySelector(".this-mac-note")?.textContent).toContain("A model larger than that still runs, but slowly");
    expect(api.hardware).toHaveBeenCalledTimes(1);
  });
  it("still says what it can when this Mac gave no facts", async () => {
    vi.mocked(api.listModels).mockResolvedValue(MODELS);
    vi.mocked(api.hardware).mockResolvedValue({ chip: null, memoryBytes: null, gpuCores: null, cpuCores: null, modelCeilingBytes: null });
    await mountModels();
    expect(host.querySelector(".this-mac")).toBeNull();
    expect(host.querySelector(".this-mac-note")?.textContent).toContain("This Mac did not say what it is made of.");
    expect(host.querySelector(".this-mac-note")?.textContent).toContain("Speed below is what each model managed last time on this Mac.");
  });
  it("has a Speed column: tokens per second when the model has answered before, a dash otherwise", async () => {
    vi.mocked(api.listModels).mockResolvedValue({ ...MODELS, speeds: { "ollama:qwen3:8b": { tokensPerSecond: 66.4, at: "2026-11-02T10:00:00Z" } } });
    await mountModels();
    expect(Array.from(host.querySelectorAll("table thead th")).map((th) => th.textContent)).toContain("Speed");
    expect(rowOf("qwen3:8b").querySelector("td.speed")?.textContent).toBe("66 tok/s");
    expect(rowOf("llama3.2:3b").querySelector("td.speed")?.textContent).toBe("—");
  });
  it("Try shows the speed beside the answer time", async () => {
    vi.mocked(api.listModels).mockResolvedValue(MODELS);
    vi.mocked(api.tryModel).mockResolvedValue({ ok: true, durationMs: 1200, answer: "pong", error: null, promptTokens: 8, completionTokens: 1, completionTokensPerSecond: 72.9 });
    await mountModels();
    const tryButton = Array.from(rowOf("qwen3:8b").querySelectorAll("button")).find((b) => b.textContent === "Try") as HTMLElement;
    await act(async () => { tryButton.click(); });
    expect(rowOf("qwen3:8b").querySelector(".try-cell")?.textContent).toContain("answered in 1.2 s · 73 tok/s · “pong”");
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

describe("Settings › Prompts", () => {
  it("has a quiet link that opens the planner's journal", async () => {
    await act(async () => { root.render(<SettingsApp />); });
    await act(async () => { (host.querySelector('nav button[aria-label="Prompts"]') as HTMLElement).click(); });
    const link = Array.from(host.querySelectorAll(".plans-journal button.link")).find((b) => b.textContent === "Open the planner's journal") as HTMLElement;
    expect(link).toBeTruthy();
    await act(async () => { link.click(); });
    expect(api.revealPlansFolder).toHaveBeenCalledTimes(1);
  });
});
