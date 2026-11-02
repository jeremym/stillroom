// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { RecipeInfo } from "../types";

vi.mock("../api", () => ({ api: { keepJournal: vi.fn(() => Promise.resolve([])) }, fmtTokens: (n: number) => String(n) }));
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

  it("a duplicate still in step with the catalog says only where it came from", () => {
    const origin = recipe("ask", { name: "Ask Your Documents" });
    const copy = recipe("ask-copy", { source: "user", shelf: "shelf", from: { catalog: "ask", version: "0.2.0", newer: false } });
    act(() => root.render(<RecipePage {...common} recipe={copy} origin={origin} onLook={() => {}} />));
    expect(host.querySelector(".provenance")?.textContent).toContain("Made from Ask Your Documents 0.2.0");
    expect(button("Look at it")).toBeUndefined();
  });
});
