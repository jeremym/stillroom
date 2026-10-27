import { describe, expect, it } from "vitest";
import { addStep, connect, dependentsOf, disconnect, removeStep, renameStepId, sourcesFor, stepInfosFromDoc, uniqueId } from "./draft";
import type { RecipeDoc } from "../types";

const doc: RecipeDoc = {
  schemaVersion: "0", id: "t", name: "T",
  inputs: { inbox: { type: "directory", label: "Inbox" }, about: { type: "text", required: false } },
  modelSlots: { triager: { requires: ["text"] } },
  steps: [
    { id: "list", kind: "tool", tool: "fs.list", inputs: { path: "${inputs.inbox}", extensions: ["eml"] } },
    {
      id: "triage", kind: "map", over: "${steps.list.output.entries}", as: "email",
      steps: [
        { id: "parse", kind: "tool", tool: "email.parse", inputs: { path: "${email.path}" } },
        { id: "classify", kind: "model", slot: "triager", prompt: "prompts/classify.md", inputs: { email: "${steps.parse.output}", about: "${inputs.about}" } },
        { id: "draft", kind: "model", slot: "triager", prompt: "prompts/draft.md", when: "${steps.classify.output.needsReply}", inputs: { email: "${steps.parse.output}" } },
      ],
    },
    { id: "table", kind: "tool", tool: "list.table", inputs: { items: "${steps.triage.output}" } },
  ],
  outputs: { triage: { from: "${steps.table.output.rows}", type: "table" } },
};

describe("stepInfosFromDoc", () => {
  it("derives dependencies and inputs like the Rust side", () => {
    const infos = stepInfosFromDoc(doc);
    expect(infos[0]).toMatchObject({ id: "list", dependsOn: [], usesInputs: ["inbox"] });
    expect(infos[1]).toMatchObject({ id: "triage", dependsOn: ["list"], usesInputs: ["about"], itemName: "email" });
    expect(infos[1].steps![2]).toMatchObject({ dependsOn: ["classify", "parse"], when: "${steps.classify.output.needsReply}" });
    expect(infos[2].dependsOn).toEqual(["triage"]);
  });
});

describe("editing", () => {
  it("adds steps with unique ids, at the top level or inside a map", () => {
    const a = addStep(doc, "model");
    expect(a.path).toBe("model");
    expect(a.doc.steps[3]).toMatchObject({ id: "model", kind: "model", slot: "triager", prompt: "prompts/model.md" });
    const b = addStep(a.doc, "tool", "triage");
    expect(b.path).toBe("triage/tool");
    expect(b.doc.steps[1].steps).toHaveLength(4);
    expect(uniqueId("list", ["list", "list_2"])).toBe("list_3");
    expect(doc.steps).toHaveLength(3);
  });

  it("connects and disconnects nodes by rewriting references", () => {
    let d = connect(doc, "in:about", "step:table", "about");
    expect(d.steps[2].inputs).toMatchObject({ about: "${inputs.about}" });
    d = connect(d, "step:triage/parse", "step:triage/draft", "parsed");
    expect(d.steps[1].steps![2].inputs).toMatchObject({ parsed: "${steps.parse.output}" });
    d = disconnect(d, "step:triage/parse", "step:triage/draft");
    expect(d.steps[1].steps![2].inputs).toEqual({});
    d = disconnect(d, "step:classify", "step:triage/draft");
    expect(d.steps[1].steps![2].when).toBeUndefined();
    d = connect(d, "step:list", "out:triage");
    expect(d.outputs.triage.from).toBe("${steps.list.output}");
    d = disconnect(d, "step:list", "step:triage");
    expect(d.steps[1].over).toBe("");
  });

  it("lists dependents and renames ids everywhere", () => {
    expect(dependentsOf(doc, "list")).toEqual(["triage"]);
    expect(dependentsOf(doc, "triage/classify")).toEqual(["triage/draft"]);
    expect(dependentsOf(doc, "table")).toEqual(["out:triage"]);
    const d = renameStepId(doc, "triage/parse", "read");
    expect(d.steps[1].steps![0].id).toBe("read");
    expect(d.steps[1].steps![1].inputs!.email).toBe("${steps.read.output}");
    const e = renameStepId(doc, "table", "summary");
    expect(e.outputs.triage.from).toBe("${steps.summary.output.rows}");
    expect(removeStep(doc, "triage/draft").steps[1].steps).toHaveLength(2);
  });

  it("offers the right sources for a step", () => {
    const top = sourcesFor(doc, "table").map((s) => s.ref);
    expect(top).toEqual(["${inputs.inbox}", "${inputs.about}", "${steps.list.output}", "${steps.triage.output}"]);
    const inner = sourcesFor(doc, "triage/draft").map((s) => s.ref);
    expect(inner).toContain("${email}");
    expect(inner).toContain("${steps.parse.output}");
    expect(inner).not.toContain("${steps.triage.output}");
    expect(inner).not.toContain("${steps.draft.output}");
  });
});
