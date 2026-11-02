// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { Grant, RecipeInfo } from "../types";

// The engine is not here; Set up's calls land on this mock instead.
vi.mock("../api", () => ({
  api: {
    estimate: vi.fn(() => Promise.resolve(null)),
    previewResolution: vi.fn(() => Promise.resolve({ resolutions: {}, problems: {} })),
    grantPermissions: vi.fn(() => Promise.resolve({})),
    revokePermissions: vi.fn(() => Promise.resolve({})),
  },
  fmtTokens: (n: number) => String(n),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
import { api } from "../api";
import { RecipePanel } from "./RecipePanel";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const home: Grant = { kind: "read", entry: "~", text: "Read every file in your home folder." };
const net: Grant = { kind: "network", entry: "api.example.org", text: "Reach api.example.org." };

function recipe(extra: Partial<RecipeInfo> = {}): RecipeInfo {
  return { id: "got", name: "A recipe someone gave me", version: "1.0.0", description: null, source: "user", shelf: "shelf", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [{ id: "make", name: "Make" }], ...extra };
}
const noop = () => {};

let host: HTMLDivElement;
let root: Root;
beforeEach(() => { host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.clearAllMocks(); });

const buttons = () => Array.from(host.querySelectorAll("button"));
const button = (text: string) => buttons().find((b) => b.textContent?.trim().includes(text));
const boxes = () => Array.from(host.querySelectorAll<HTMLInputElement>(".consent-zone input[type=checkbox]"));
const tick = (box: HTMLInputElement) => act(() => { box.click(); });
const settle = () => act(async () => { await Promise.resolve(); });

function render(r: RecipeInfo, onConsentChanged = noop) {
  act(() => root.render(
    <RecipePanel recipes={[r]} recipe={r} onSelect={noop} pins={{}} onPins={noop} models={[]} running={false}
      onRun={noop} onWatch={noop} onConsentChanged={onConsentChanged} />,
  ));
}

describe("the consent step in Set up", () => {
  it("a built-in with nothing to allow shows no zone and Run is ready", async () => {
    render(recipe({ source: "builtin" }));
    await settle();
    expect(host.querySelector(".consent-zone")).toBeNull();
    expect(host.textContent).toContain("Permissions:");
    expect((button("Run") as HTMLButtonElement).disabled).toBe(false);
  });

  it("Run and Watch wait until every line is ticked, then the grants are written", async () => {
    const changed = vi.fn();
    render(recipe({ consent: { needed: [home, net], granted: [] }, inputs: { folder: { type: "directory", label: "Folder" } }, stages: [{ id: "make", name: "Make", inputs: ["folder"] }] }), changed);
    await settle();
    expect(host.textContent).toContain("What this recipe may touch");
    expect(host.textContent).toContain(home.text);
    expect(host.textContent).toContain("Tick what you allow. The recipe will not run until every line is ticked.");
    const run = button("Run") as HTMLButtonElement;
    expect(run.textContent).toContain("needs your permission");
    expect(run.disabled).toBe(true);
    expect((button("Watch") as HTMLButtonElement).disabled).toBe(true);

    tick(boxes()[0]);
    expect(api.grantPermissions).not.toHaveBeenCalled();
    expect((button("Run") as HTMLButtonElement).disabled).toBe(true);

    tick(boxes()[1]);
    expect(api.grantPermissions).toHaveBeenCalledWith("got", [home, net]);
    await settle();
    expect(changed).toHaveBeenCalled();
  });

  it("once allowed, the lines show set with Take back; Run waits only on the inputs", async () => {
    const changed = vi.fn();
    render(recipe({ consent: { needed: [], granted: [{ ...home, grantedAt: "2026-11-02T10:00:00Z" }] } }), changed);
    await settle();
    const [box] = boxes();
    expect(box.checked).toBe(true);
    expect(box.disabled).toBe(true);
    expect(host.textContent).toMatch(/allowed on /);
    expect((button("Run") as HTMLButtonElement).disabled).toBe(false);
    expect(button("Run")?.textContent).not.toContain("permission");

    act(() => button("Take back")!.click());
    expect(api.revokePermissions).toHaveBeenCalledWith("got");
    await settle();
    expect(changed).toHaveBeenCalled();
  });
});
