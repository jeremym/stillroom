// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { InstalledModel, RecipeInfo } from "../types";

// The engine is not here; the place's calls land on this mock instead.
vi.mock("../api", () => ({ api: { setShelf: vi.fn(() => Promise.resolve()), revealSampleFolders: vi.fn(() => Promise.resolve()) } }));
import { api } from "../api";
import { CatalogCard, CatalogPlace } from "./Catalog";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function recipe(id: string, extra: Partial<RecipeInfo> = {}): RecipeInfo {
  return { id, name: `Recipe ${id}`, version: "0.1.0", description: "A line about it.", source: "builtin", shelf: "catalog", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [], tags: ["built-in"], ...extra };
}
const text: InstalledModel = { id: "qwen", provider: "ollama", name: "qwen", capabilities: ["text"], contextWindow: null, parameterSize: null, local: true };
const facts = { models: [text], history: [] };

const docs = recipe("ask", { name: "Ask Your Documents", tags: ["built-in", "documents"],
  inputs: { folder: { type: "directory", label: "Folder" }, question: { type: "string", label: "Question" } },
  modelSlots: { writer: { requires: ["text"] }, embedder: { requires: ["embedding"] } },
  permissions: { filesystem: { read: ["input"], write: ["output"] } } });
const brief = recipe("brief", { name: "Research Brief", tags: ["built-in", "research"], shelf: "shelf",
  inputs: { links: { type: "urls", label: "Links" } }, permissions: { network: ["input"] } });
const sample = recipe("inbox", { name: "Inbox Triage", source: "sample", tags: ["sample", "email"] });
const community = recipe("notes", { name: "Meeting Notes", source: "community", tags: ["documents"], author: { name: "Someone" } });
const mine = recipe("mine", { source: "user", shelf: "shelf" });

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
const button = (text: string) => buttons().find((b) => b.textContent?.trim().includes(text));
const headings = () => Array.from(host.querySelectorAll("h2")).map((h) => h.textContent ?? "");
const cardNames = () => Array.from(host.querySelectorAll(".card-title")).map((t) => t.textContent?.trim());

describe("CatalogCard", () => {
  it("says what it needs and may touch, adds to the shelf, and opens Look", async () => {
    const onLook = vi.fn();
    const onAdd = vi.fn();
    act(() => root.render(<CatalogCard recipe={docs} facts={facts} onLook={onLook} onAdd={onAdd} />));
    const lines = host.querySelector(".card-lines")?.textContent ?? "";
    expect(lines).toContain("needs a folder, a question");
    expect(lines).toContain("may touch reads the files and folders you choose · writes the run's own folder · no network");
    // Two slots, one installed model that fits: the card warns.
    expect(host.textContent).toContain("needs a model");

    click(button("Add to shelf"));
    expect(onAdd).toHaveBeenCalledWith("ask");
    click(button("Look"));
    expect(onLook).toHaveBeenCalledWith("ask");
    click(host.querySelector("button.card"));
    expect(onLook).toHaveBeenCalledTimes(2);
  });

  it("shows on the shelf instead of Add to shelf once shelved", () => {
    act(() => root.render(<CatalogCard recipe={brief} facts={facts} onLook={() => {}} onAdd={() => {}} />));
    expect(button("Add to shelf")).toBeUndefined();
    expect(host.querySelector(".card-foot .tag")?.textContent).toBe("on the shelf");
    expect(button("Look")).toBeTruthy();
  });
});

describe("CatalogPlace", () => {
  const all = [docs, brief, sample, community, mine];

  it("lays out the three rows, leaves your recipes out, and names the community author", () => {
    act(() => root.render(<CatalogPlace recipes={all} facts={facts} onLook={() => {}} onError={() => {}} />));
    expect(headings().some((h) => h.startsWith("Built-in"))).toBe(true);
    expect(headings().some((h) => h.startsWith("Community"))).toBe(true);
    expect(headings().some((h) => h.startsWith("Samples"))).toBe(true);
    expect(cardNames()).toEqual(["Ask Your Documents", "Research Brief", "Meeting Notes", "Inbox Triage"]);
    expect(host.textContent).toContain("by Someone");
    expect(host.textContent).toContain("reviewed before it shipped");
    expect(host.textContent).toContain("copy them first");
  });

  it("Add to shelf asks the engine; Show the sample folders reveals them", async () => {
    const onChanged = vi.fn();
    act(() => root.render(<CatalogPlace recipes={all} facts={facts} onLook={() => {}} onError={() => {}} onChanged={onChanged} />));
    click(button("Add to shelf"));
    expect(api.setShelf).toHaveBeenCalledWith("ask", "shelf");
    await act(async () => { await Promise.resolve(); });
    expect(onChanged).toHaveBeenCalled();
    click(button("Show the sample folders"));
    expect(api.revealSampleFolders).toHaveBeenCalled();
  });

  it("filters by tag, by what runs here, and by search, hiding empty rows", () => {
    act(() => root.render(<CatalogPlace recipes={all} facts={facts} onLook={() => {}} onError={() => {}} />));
    const kinds = host.querySelector(".segmented[aria-label='Kind of recipe']")!;
    const kind = (t: string) => Array.from(kinds.querySelectorAll("button")).find((b) => b.textContent === t);
    expect(Array.from(kinds.querySelectorAll("button")).map((b) => b.textContent)).toEqual(["all", "documents", "research", "email"]);

    click(kind("documents"));
    expect(cardNames()).toEqual(["Ask Your Documents", "Meeting Notes"]);
    expect(headings().some((h) => h.startsWith("Samples"))).toBe(false);

    click(kind("all"));
    click(host.querySelector(".toggle"));
    // Ask Your Documents needs an embedding model nobody has installed.
    expect(cardNames()).toEqual(["Research Brief", "Meeting Notes", "Inbox Triage"]);
    click(host.querySelector(".toggle"));

    const search = host.querySelector<HTMLInputElement>("input[type=search]")!;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(search, "triage");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(cardNames()).toEqual(["Inbox Triage"]);

    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(search, "nothing like this");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(cardNames()).toEqual([]);
    expect(host.textContent).toContain("Nothing matches");
    click(button("Clear the filters"));
    expect(cardNames()).toHaveLength(4);
  });
});
