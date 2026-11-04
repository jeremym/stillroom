// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { Draft, EditOutcome, RecipeInfo } from "../types";

const readRecipeDraft = vi.fn<() => Promise<{ draft: Draft }>>();
const editRecipe = vi.fn<() => Promise<EditOutcome>>();
const saveRecipeDraft = vi.fn<(draft: Draft, expectNew: boolean) => Promise<{ recipe: RecipeInfo }>>();
vi.mock("../api", () => ({
  api: {
    readRecipeDraft: () => readRecipeDraft(),
    editRecipe: () => editRecipe(),
    saveRecipeDraft: (d: Draft, n: boolean) => saveRecipeDraft(d, n),
    stopRun: vi.fn(() => Promise.resolve(true)),
  },
}));
import { EMPTY_STREAM } from "../planStream";
import { AskForChange, changedId, rewrittenFiles } from "./AskForChange";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const before: Draft = { yaml: { schemaVersion: "0", id: "learn", name: "Learn", steps: [], outputs: {} }, files: { "prompts/brief.md": "Write the brief.", "prompts/quiz.md": "Write a quiz." } };
const after: Draft = { yaml: { schemaVersion: "0", id: "learn", name: "Learn", steps: [], outputs: {} }, files: { "prompts/brief.md": "Write the brief.", "prompts/quiz.md": "Write a quiz from the brief." } };
function recipe(source: RecipeInfo["source"] = "user"): RecipeInfo {
  return { id: "learn", name: "Learn", version: "0.1.0", description: null, source, shelf: "shelf", path: "", inputs: {}, modelSlots: {}, steps: [], outputs: {}, stages: [] };
}
function outcome(changes: string[], problems: string[] = []): EditOutcome {
  return { draft: after, problems, attempts: 1, modelId: "ollama:q", promptTokens: 10, completionTokens: 5, durationMs: 4200, transcript: "── call 1 (steps) ──\n{}", changes };
}

let host: HTMLDivElement;
let root: Root;
beforeEach(() => { host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host); readRecipeDraft.mockResolvedValue({ draft: before }); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.clearAllMocks(); });

const button = (text: string) => Array.from(host.querySelectorAll("button")).find((b) => b.textContent?.trim().includes(text)) as HTMLButtonElement | undefined;
const settle = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });
function type(text: string) {
  const ta = host.querySelector("textarea")!;
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(ta, text);
    ta.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("Ask for a change", () => {
  it("plans from an instruction, lists what changed with a prompt before and after, and saves a copy under a free id", async () => {
    editRecipe.mockResolvedValue(outcome(["Step changed: Write the quiz (`quiz`).", "File rewritten: `prompts/quiz.md`."]));
    saveRecipeDraft.mockResolvedValue({ recipe: { ...recipe(), id: "learn-changed-2" } });
    const saved = vi.fn();
    act(() => root.render(<AskForChange recipe={recipe()} planStream={EMPTY_STREAM} takenIds={["learn", "learn-changed"]} onSaved={saved} />));
    expect(button("Plan the change")?.disabled).toBe(true);
    type("The quiz should come from the brief.");
    expect(button("Plan the change")?.disabled).toBe(false);
    act(() => { button("Plan the change")!.click(); });
    await settle();
    expect(editRecipe).toHaveBeenCalledTimes(1);
    const items = Array.from(host.querySelectorAll(".change-list li")).map((li) => li.textContent);
    expect(items).toEqual(["Step changed: Write the quiz (`quiz`).", "File rewritten: `prompts/quiz.md`."]);
    expect(host.querySelector(".change-file summary")?.textContent).toBe("prompts/quiz.md · before and after");
    expect(host.querySelector(".change-before")?.textContent).toBe("Write a quiz.");
    expect(host.querySelector(".change-after")?.textContent).toBe("Write a quiz from the brief.");
    expect(host.textContent).toContain("Changed by q (local) in 4 s.");
    // One of your own: both ways to save; the copy takes the next free id and says so in its name.
    expect(button("Save in place")).toBeDefined();
    act(() => { button("Save as a copy")!.click(); });
    await settle();
    expect(saveRecipeDraft).toHaveBeenCalledTimes(1);
    const [draft, expectNew] = saveRecipeDraft.mock.calls[0];
    expect(expectNew).toBe(true);
    expect(draft.yaml).toMatchObject({ id: "learn-changed-2", name: "Learn (changed)" });
    expect(draft.files["prompts/quiz.md"]).toBe("Write a quiz from the brief.");
    expect(saved).toHaveBeenCalledWith(expect.objectContaining({ id: "learn-changed-2" }));
  });

  it("never offers to change a built-in in place, and a change with problems saves nothing", async () => {
    editRecipe.mockResolvedValue(outcome([], ["Result “cards” comes from a step that does not exist: cards."]));
    act(() => root.render(<AskForChange recipe={recipe("builtin")} planStream={EMPTY_STREAM} takenIds={[]} onSaved={vi.fn()} />));
    type("Drop the voice script.");
    act(() => { button("Plan the change")!.click(); });
    await settle();
    expect(host.querySelector(".notice")?.textContent).toContain("nothing was saved");
    expect(host.querySelector(".plan-problems-list")?.textContent).toContain("comes from a step that does not exist");
    expect(button("Save as a copy")).toBeUndefined();
    expect(button("Save in place")).toBeUndefined();
    editRecipe.mockResolvedValue(outcome(["Step removed: Read it aloud (`say`)."]));
    act(() => { button("Plan the change")!.click(); });
    await settle();
    expect(button("Save as a copy")).toBeDefined();
    expect(button("Save in place")).toBeUndefined();
    expect(host.textContent).toContain("A built-in is never changed in place");
  });

  it("names a copy after the recipe and picks a free id", () => {
    expect(changedId("learn", [])).toBe("learn-changed");
    expect(changedId("learn", ["learn-changed"])).toBe("learn-changed-2");
    expect(changedId("learn", ["learn-changed", "learn-changed-2"])).toBe("learn-changed-3");
    expect(rewrittenFiles(before, after)).toEqual([["prompts/quiz.md", "Write a quiz.", "Write a quiz from the brief."]]);
  });
});
