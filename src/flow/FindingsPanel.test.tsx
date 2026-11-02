// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { Draft } from "../types";

vi.mock("../api", () => ({ api: { fixRecipeDraft: vi.fn() } }));
import { api } from "../api";
import { FindingsPanel } from "./FindingsPanel";

const draft: Draft = { yaml: { id: "r", name: "R", steps: [], outputs: {} } as unknown as Draft["yaml"], files: {} };
const fixed: Draft = { yaml: { ...draft.yaml, fixedBy: "engine" } as unknown as Draft["yaml"], files: { "a.md": "" } };

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  vi.mocked(api.fixRecipeDraft).mockReset();
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const buttons = () => [...host.querySelectorAll("button")];
const byLabel = (label: string) => buttons().find((b) => b.textContent === label);

describe("FindingsPanel", () => {
  it("shows problems under 'Must be fixed before saving' and notes under 'Worth a look'", async () => {
    await act(async () => {
      root.render(<FindingsPanel draft={draft} onDraft={() => {}} findings={[
        { text: "steps[0] has no model", severity: "problem" },
        { text: "This recipe uses `x`, which is now `y`.", severity: "note" },
      ]} />);
    });
    expect(host.textContent).toContain("Must be fixed before saving");
    expect(host.textContent).toContain("steps[0] has no model");
    expect(host.textContent).toContain("Worth a look");
    expect(host.textContent).toContain("This recipe uses `x`, which is now `y`.");
    expect(buttons()).toHaveLength(0);
  });

  it("renders a note's fix button with its label and applies the fix through the engine", async () => {
    vi.mocked(api.fixRecipeDraft).mockResolvedValue(fixed);
    const onDraft = vi.fn();
    await act(async () => {
      root.render(<FindingsPanel draft={draft} onDraft={onDraft} findings={[
        { text: "Stage `index` writes a keep but does not declare one.", severity: "note", fix: "declare-keeps", fixLabel: "Declare it" },
      ]} />);
    });
    const button = byLabel("Declare it");
    expect(button).toBeDefined();
    expect(byLabel("Fix all")).toBeUndefined();
    await act(async () => { button!.click(); });
    expect(api.fixRecipeDraft).toHaveBeenCalledWith(draft, ["declare-keeps"]);
    expect(onDraft).toHaveBeenCalledWith(fixed);
  });

  it("falls back to 'Fix it' and offers 'Fix all' when more than one note has a fix", async () => {
    vi.mocked(api.fixRecipeDraft).mockResolvedValue(fixed);
    const onDraft = vi.fn();
    await act(async () => {
      root.render(<FindingsPanel draft={draft} onDraft={onDraft} findings={[
        { text: "old a", severity: "note", fix: "a" },
        { text: "old b", severity: "note", fix: "b", fixLabel: "" },
      ]} />);
    });
    expect(buttons().filter((b) => b.textContent === "Fix it")).toHaveLength(2);
    await act(async () => { byLabel("Fix all")!.click(); });
    expect(api.fixRecipeDraft).toHaveBeenCalledWith(draft, ["a", "b"]);
    expect(onDraft).toHaveBeenCalledWith(fixed);
  });

  it("keeps the draft and says so when the engine refuses the fix", async () => {
    vi.mocked(api.fixRecipeDraft).mockRejectedValue("unknown fix: nope");
    const onDraft = vi.fn();
    await act(async () => {
      root.render(<FindingsPanel draft={draft} onDraft={onDraft} findings={[{ text: "old", severity: "note", fix: "nope" }]} />);
    });
    await act(async () => { byLabel("Fix it")!.click(); });
    expect(onDraft).not.toHaveBeenCalled();
    expect(host.textContent).toContain("The fix did not apply: unknown fix: nope");
  });
});
