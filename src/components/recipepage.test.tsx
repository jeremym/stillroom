// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { RecipeInfo } from "../types";

const exampleRun = {
  id: "ex", recipeId: "ask", recipeName: "Recipe ask", status: "succeeded", startedAt: "2026-11-02T10:00:00Z", finishedAt: "2026-11-02T10:00:09Z",
  inputs: {}, resolutions: { writer: { slot: "writer", modelId: "ollama:qwen3:8b", via: "capability" } }, steps: [], outputs: { summary: "# A summary\n\nTwo lines about the document." }, error: null,
};
vi.mock("../api", () => ({ api: { keepJournal: vi.fn(() => Promise.resolve([])), exampleResult: vi.fn(() => Promise.resolve(exampleRun)) }, fmtTokens: (n: number) => String(n) }));
import { api } from "../api";
import { RecipePage } from "./RecipePage";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function recipe(id: string, extra: Partial<RecipeInfo> = {}): RecipeInfo {
  return { id, name: `Recipe ${id}`, version: "0.2.0", description: "A line about it.", source: "builtin", shelf: "catalog", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [], ...extra };
}
const noop = () => {};
const common = { models: [], history: [], watches: [], map: null, onSetup: noop, onDoor: noop, onOpenRun: noop, onDuplicate: noop, onRename: noop, onDelete: noop, onWatchesChanged: noop, onError: noop };

let host: HTMLDivElement;
let root: Root;
beforeEach(() => { host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const buttons = () => Array.from(host.querySelectorAll("button"));
const button = (text: string) => buttons().find((b) => b.textContent?.trim().includes(text));

describe("RecipePage", () => {
  it("in the catalog's Look: Add to shelf stands first and nothing runs or copies", () => {
    const onAdd = vi.fn();
    act(() => root.render(<RecipePage {...common} recipe={recipe("ask")} readOnly onAddToShelf={onAdd} />));
    expect(button("Set up a run")).toBeUndefined();
    expect(button("Duplicate")).toBeUndefined();
    expect(button("Recipe card")).toBeUndefined();
    expect(button("Open the recipe card to edit")).toBeUndefined();
    expect(host.textContent).toContain("in the catalog");
    act(() => button("Add to shelf")!.click());
    expect(onAdd).toHaveBeenCalled();
  });

  it("shelved: the normal buttons are back", () => {
    act(() => root.render(<RecipePage {...common} recipe={recipe("ask", { shelf: "shelf" })} />));
    expect(button("Set up a run")).toBeTruthy();
    expect(button("Duplicate")).toBeTruthy();
    expect(button("Add to shelf")).toBeUndefined();
  });

  it("a duplicate says where it came from and points at the newer catalog recipe", () => {
    const onLook = vi.fn();
    const origin = recipe("ask", { name: "Ask Your Documents", version: "0.3.0" });
    const copy = recipe("ask-copy", { source: "user", shelf: "shelf", from: { catalog: "ask", version: "0.2.0", newer: true } });
    act(() => root.render(<RecipePage {...common} recipe={copy} origin={origin} onLook={onLook} />));
    const line = host.querySelector(".provenance")?.textContent ?? "";
    expect(line).toContain("Made from Ask Your Documents 0.2.0");
    expect(line).toContain("the catalog now has 0.3.0");
    act(() => button("Look at it")!.click());
    expect(onLook).toHaveBeenCalledWith("ask");
  });

  it("See an example result loads the saved run and shows it read-only, on the Look page too", async () => {
    const r = recipe("ask", { example: true, outputs: { summary: { from: "${steps.write.output}", type: "markdown", label: "Summary" } } });
    act(() => root.render(<RecipePage {...common} recipe={r} readOnly onAddToShelf={() => {}} />));
    expect(button("Add to shelf")).toBeTruthy();
    act(() => button("See an example result")!.click());
    expect(api.exampleResult).toHaveBeenCalledWith("ask");
    await act(async () => { await Promise.resolve(); });
    expect(host.textContent).toContain("An example result");
    expect(host.textContent).toContain("You are looking at a saved example, not a run on your files.");
    expect(host.textContent).toContain("with qwen3:8b");
    expect(host.textContent).toContain("Two lines about the document.");
    // Look only: nothing to edit or save.
    expect(button("Edit…")).toBeUndefined();
    expect(button("Save…")).toBeUndefined();
    act(() => button("Hide the example")!.click());
    expect(host.textContent).not.toContain("An example result");
  });

  it("without an example there is no button for one", () => {
    act(() => root.render(<RecipePage {...common} recipe={recipe("ask", { shelf: "shelf" })} />));
    expect(button("See an example result")).toBeUndefined();
  });

  it("a duplicate still in step with the catalog says only where it came from", () => {
    const origin = recipe("ask", { name: "Ask Your Documents" });
    const copy = recipe("ask-copy", { source: "user", shelf: "shelf", from: { catalog: "ask", version: "0.2.0", newer: false } });
    act(() => root.render(<RecipePage {...common} recipe={copy} origin={origin} onLook={() => {}} />));
    expect(host.querySelector(".provenance")?.textContent).toContain("Made from Ask Your Documents 0.2.0");
    expect(button("Look at it")).toBeUndefined();
  });
});
