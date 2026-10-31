import { useEffect, useState } from "react";
import { Glyph } from "../brand";
import { InputField } from "./RecipePanel";
import type { RecipeInfo, Run } from "../types";

/* Ask again: on Results, the inputs of the last stage that asks for something (a question, a new
   file) with the run's values filled in, and one button that runs the recipe again with only
   those changed. Everything earlier is kept or reused, so this is the fast loop. */

interface Props {
  recipe: RecipeInfo;
  run: Run;
  running: boolean;
  onRun: (inputs: Record<string, unknown>) => void;
}

export function askAgainStage(recipe: RecipeInfo): RecipeInfo["stages"][number] | null {
  const candidates = recipe.stages.filter((st, i) => i > 0 && (st.inputs ?? []).some((n) => n in recipe.inputs));
  return candidates.length ? candidates[candidates.length - 1] : null;
}

export function AskAgain({ recipe, run, running, onRun }: Props) {
  const stage = askAgainStage(recipe);
  const [values, setValues] = useState<Record<string, unknown>>({});
  useEffect(() => { setValues({}); }, [run.id]);
  if (!stage) return null;
  const names = (stage.inputs ?? []).filter((n) => n in recipe.inputs);
  const merged = { ...run.inputs, ...values };
  const missing = names.filter((n) => recipe.inputs[n]?.required !== false && (merged[n] == null || merged[n] === ""));
  const kept = recipe.stages.filter((st) => st.keep && recipe.stages.indexOf(st) < recipe.stages.indexOf(stage)).map((st) => st.name);
  return (
    <section className="panel ask-again">
      <h3><Glyph name="sparkles" /> {stage.name} again</h3>
      <p className="muted">
        Change only this and run again. {kept.length ? `${kept.join(" and ")} ${kept.length === 1 ? "is" : "are"} kept, so only the later steps run.` : "Earlier steps whose inputs did not change are reused."}
      </p>
      {names.map((n) => (
        <InputField key={n} name={n} spec={recipe.inputs[n]} value={merged[n]} onChange={(v) => setValues((x) => ({ ...x, [n]: v }))} />
      ))}
      <button className="primary" style={{ marginTop: 0 }} disabled={running || missing.length > 0} onClick={() => onRun(merged)}>
        <Glyph name="power" /> {running ? "Running…" : missing.length ? `Run · needs ${missing.join(", ")}` : "Run again"}
      </button>
    </section>
  );
}
