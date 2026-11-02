// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { Grant, ImportPreview, RecipeInfo } from "../types";

vi.mock("../api", () => ({ api: { keepJournal: vi.fn(() => Promise.resolve([])), fixRecipeDraft: vi.fn() }, fmtTokens: (n: number) => String(n) }));
import { ImportPlace } from "./ImportPlace";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const home: Grant = { kind: "read", entry: "~", text: "Read every file in your home folder." };
const shell: Grant = { kind: "shell", entry: "shell", text: "Run programs on this Mac." };

function info(extra: Partial<RecipeInfo> = {}): RecipeInfo {
  return { id: "got", name: "A recipe someone gave me", version: "1.0.0", description: "Does a thing.", source: "user", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [], ...extra };
}
function preview(extra: Partial<ImportPreview> = {}): ImportPreview {
  return { token: "t1", info: info({ consent: { needed: [home, shell], granted: [] } }), findings: [], sizeBytes: 12 * 1024, ...extra };
}

let host: HTMLDivElement;
let root: Root;
beforeEach(() => { host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const buttons = () => Array.from(host.querySelectorAll("button"));
const button = (text: string) => buttons().find((b) => b.textContent?.trim().includes(text)) as HTMLButtonElement | undefined;
const boxes = () => Array.from(host.querySelectorAll<HTMLInputElement>(".consent-zone input[type=checkbox]"));
const tick = (box: HTMLInputElement) => act(() => { box.click(); });

function render(p: ImportPreview, onConfirm = vi.fn(), onDiscard = vi.fn()) {
  act(() => root.render(<ImportPlace preview={p} models={[]} busy={false} onConfirm={onConfirm} onDiscard={onDiscard} onError={() => {}} />));
  return { onConfirm, onDiscard };
}

describe("Get a recipe: the import place", () => {
  it("shows the recipe read-only with nothing that runs, and its size", () => {
    render(preview());
    expect(host.textContent).toContain("A recipe someone gave me");
    expect(host.textContent).toContain("not added yet");
    expect(host.textContent).toContain("12 KB");
    expect(button("Set up a run")).toBeUndefined();
    expect(button("Add to shelf")).toBeUndefined();
    expect(button("Duplicate")).toBeUndefined();
    expect(host.textContent).not.toContain("Runs of this recipe");
  });

  it("Add waits until every line is ticked, then sends the ticked grants", () => {
    const { onConfirm } = render(preview());
    expect(button("Add to my recipes")!.disabled).toBe(true);
    expect(host.textContent).toContain("tick every line above first");
    tick(boxes()[0]);
    expect(button("Add to my recipes")!.disabled).toBe(true);
    tick(boxes()[1]);
    expect(button("Add to my recipes")!.disabled).toBe(false);
    act(() => button("Add to my recipes")!.click());
    expect(onConfirm).toHaveBeenCalledWith([home, shell]);
  });

  it("a problem blocks Add even with every line ticked; a note does not", () => {
    render(preview({ findings: [{ text: "Step read names a file that is not in the bundle.", severity: "problem" }, { text: "Uses an older shape.", severity: "note", fix: "x", fixLabel: "Fix it" }] }));
    tick(boxes()[0]);
    tick(boxes()[1]);
    expect(button("Add to my recipes")!.disabled).toBe(true);
    expect(host.textContent).toContain("Stops the import");
    expect(host.textContent).toContain("1 problem must be fixed");
    // Nothing can be fixed from here: the finding's Fix button is not offered.
    expect(button("Fix it")).toBeUndefined();
  });

  it("nothing to allow: Add is ready at once; Discard says so", () => {
    const { onConfirm, onDiscard } = render(preview({ info: info() }));
    expect(host.textContent).toContain("Nothing to allow");
    expect(button("Add to my recipes")!.disabled).toBe(false);
    act(() => button("Add to my recipes")!.click());
    expect(onConfirm).toHaveBeenCalledWith([]);
    act(() => button("Discard")!.click());
    expect(onDiscard).toHaveBeenCalled();
  });
});
