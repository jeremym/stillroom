// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { RecipeInfo } from "../types";

// The engine is not here; the zone's calls land on this mock instead.
vi.mock("../api", () => ({ api: { setShelf: vi.fn(() => Promise.resolve()), revealRecipesFolder: vi.fn(() => Promise.resolve()) } }));
import { api } from "../api";
import { ArchiveZone, CatalogZone } from "./Shelf";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function recipe(id: string, extra: Partial<RecipeInfo> = {}): RecipeInfo {
  return { id, name: `Recipe ${id}`, version: null, description: "A line about it.", source: "builtin", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [], ...extra };
}
const facts = { models: [], history: [] };

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
const buttons = () => Array.from(host.querySelectorAll("button"));
const button = (text: string) => buttons().find((b) => b.textContent?.includes(text));

describe("ArchiveZone", () => {
  it("renders nothing when the archive is empty", () => {
    act(() => root.render(<ArchiveZone recipes={[]} facts={facts} onOpen={() => {}} onError={() => {}} />));
    expect(host.innerHTML).toBe("");
  });

  it("folds by default, shows the count, and Restore puts a recipe back on the shelf", async () => {
    const onChanged = vi.fn();
    act(() => root.render(<ArchiveZone recipes={[recipe("a", { shelf: "archive" }), recipe("b", { shelf: "archive" })]} facts={facts} onOpen={() => {}} onError={() => {}} onChanged={onChanged} />));
    expect(host.querySelector("h2")?.textContent).toContain("Archive");
    expect(button("Show (2)")).toBeTruthy();
    expect(button("Restore")).toBeUndefined();

    click(button("Show (2)"));
    const restores = buttons().filter((b) => b.textContent?.includes("Restore"));
    expect(restores).toHaveLength(2);
    // Nothing else acts on an archived card: no menu, no Add to shelf.
    expect(host.querySelector(".card-menu")).toBeNull();
    expect(button("Add to shelf")).toBeUndefined();

    click(restores[1]);
    expect(api.setShelf).toHaveBeenCalledWith("b", "shelf");
    await act(async () => { await Promise.resolve(); });
    expect(onChanged).toHaveBeenCalled();
  });
});

describe("CatalogZone", () => {
  it("adds a recipe to the shelf and opens the card", () => {
    const onOpen = vi.fn();
    act(() => root.render(<CatalogZone recipes={[recipe("c", { shelf: "catalog" })]} facts={facts} open onToggle={() => {}} onOpen={onOpen} onError={() => {}} />));
    click(button("Add to shelf"));
    expect(api.setShelf).toHaveBeenCalledWith("c", "shelf");
    click(host.querySelector("button.card"));
    expect(onOpen).toHaveBeenCalledWith("c");
  });
});
