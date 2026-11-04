// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { PermissionRow, RecipeInfo } from "../types";

// The engine is not here; Set up's calls land on this mock instead.
const probe = vi.fn<(id: string, inputs: Record<string, unknown>) => Promise<PermissionRow[]>>(() => Promise.resolve([]));
vi.mock("../api", () => ({
  api: {
    estimate: vi.fn(() => Promise.resolve(null)),
    previewResolution: vi.fn(() => Promise.resolve({ resolutions: {}, problems: {} })),
    probePermissions: (id: string, inputs: Record<string, unknown>) => probe(id, inputs),
  },
  fmtTokens: (n: number) => String(n),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
import { RecipePanel } from "./RecipePanel";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function recipe(): RecipeInfo {
  return {
    id: "learn", name: "Learn stuff", version: "0.1.0", description: null, source: "user", shelf: "shelf", path: "",
    inputs: {
      urls: { type: "urls", label: "Pages to read", description: "One link per line.", required: false },
      folder: { type: "directory", label: "Your documents", description: "A folder of .pdf or .md files." },
      focus: { type: "text", label: "What do you need to know?", description: "A question or a sentence." },
      out: { type: "directory", label: "Where to save" },
    },
    modelSlots: {}, steps: [], outputs: {},
    stages: [
      { id: "links", name: "Read links", why: "Each link is fetched and read.", inputs: ["urls"] },
      { id: "docs", name: "Read documents", why: "Every file in the folder is read.", inputs: ["folder", "focus"] },
      { id: "brief", name: "Write the brief", why: "One brief from everything read." },
      { id: "save", name: "Save it", why: "Written as files into the folder you choose.", inputs: ["out"] },
    ],
  };
}
const noop = () => {};

let host: HTMLDivElement;
let root: Root;
beforeEach(() => { vi.useFakeTimers(); host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.clearAllMocks(); vi.useRealTimers(); });

const group = (name: string) => host.querySelector(`[role=group][aria-label="${name}"]`) as HTMLElement | null;
const run = () => Array.from(host.querySelectorAll("button")).find((b) => b.textContent?.trim().startsWith("Run")) as HTMLButtonElement;
const settle = () => act(async () => { await vi.advanceTimersByTimeAsync(300); });

function render(r: RecipeInfo) {
  act(() => root.render(<RecipePanel recipes={[r]} recipe={r} onSelect={noop} pins={{}} onPins={noop} models={[]} running={false} onRun={noop} onWatch={noop} />));
}

describe("Set up as stage groups", () => {
  it("numbers each stage with inputs, puts the why under the head and the description under the control, and rims what still needs you", async () => {
    render(recipe());
    await settle();
    // Stages with inputs are groups; the brief stage, with none, is not shown.
    expect(Array.from(host.querySelectorAll(".group .group-n")).map((n) => n.textContent)).toEqual(["1", "2", "4"]);
    expect(group("Write the brief")).toBeNull();
    const docs = group("Read documents")!;
    expect(docs.className).toBe("group amber-rim stage-section");
    expect(docs.querySelector(".group-why")?.textContent).toBe("Every file in the folder is read.");
    expect(docs.querySelector(".group-head h3")?.textContent).toBe("Read documents");
    // The field's description sits under its control, not on the label line; a required label carries the mark.
    const field = docs.querySelector(".field")!;
    expect(field.querySelector("label")?.textContent).toBe("Your documents*");
    expect(field.querySelector("label + .row + .desc")?.textContent).toBe("A folder of .pdf or .md files.");
    // An all-optional stage says so in the head and has no rim.
    const links = group("Read links")!;
    expect(links.className).toBe("group optional stage-section");
    expect(links.querySelector(".group-head h3")?.textContent).toBe("Read links (optional)");
    expect(links.querySelector(".field label")?.textContent).toBe("Pages to read (optional)");
    // Run says which stage needs what.
    expect(run().textContent).toBe("Run · stage 2 needs Your documents, What do you need to know?; stage 4 needs Where to save");
    // Three or more stages empty: one short line, not a paragraph.
    const r = recipe();
    r.inputs.urls = { ...r.inputs.urls, required: true };
    render(r);
    await settle();
    expect(run().textContent).toBe("Run · needs you in stages 1, 2, 4");
    expect(run().disabled).toBe(true);
  });

  it("drops the rim as a stage fills in, and Run names only what is left", async () => {
    render(recipe());
    await settle();
    const focus = group("Read documents")!.querySelector("textarea")!;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(focus, "graph theory");
      focus.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await settle();
    expect(run().textContent).toBe("Run · stage 2 needs Your documents; stage 4 needs Where to save");
    expect(group("Read documents")!.className).toContain("amber-rim");
  });

  it("shows the permission rows the engine probed, and a folder that cannot be written holds Run", async () => {
    probe.mockResolvedValue([
      { kind: "read", label: "the files and folders you choose", detail: "~/Reading", status: "ok", note: null },
      { kind: "write", label: "Where to save", detail: "~/Study", status: "cannot_write", note: "Stillroom cannot write in ~/Study; choose another folder" },
      { kind: "reach", label: "the sites in the links you give it", detail: null, status: "not_chosen", note: "no links yet" },
    ]);
    const r = recipe();
    for (const k of Object.keys(r.inputs)) r.inputs[k] = { ...r.inputs[k], required: false };
    render(r);
    await settle();
    expect(probe).toHaveBeenCalledWith("learn", expect.any(Object));
    const rows = Array.from(host.querySelectorAll(".perm-row")).map((li) => Array.from(li.childNodes).map((n) => n.textContent?.trim()).filter(Boolean).join(" "));
    expect(rows[0]).toBe("Reads the files and folders you choose · ~/Reading");
    expect(rows[1]).toContain("Writes Where to save · ~/Study cannot write");
    expect(rows[1]).toContain("choose another folder");
    expect(rows[2]).toBe("Reaches the sites in the links you give it not chosen yet");
    expect(rows[3]).toBe("Nothing else is touched.");
    expect(run().textContent).toBe("Run · cannot write Where to save");
    expect(run().disabled).toBe(true);
  });

  it("falls back to the one-line description when the engine cannot be asked", async () => {
    probe.mockRejectedValue(new Error("no engine"));
    render(recipe());
    await settle();
    expect(host.querySelector(".perm-list")).toBeNull();
    expect(host.textContent).toContain("Permissions: reads the files and folders you choose");
  });
});
