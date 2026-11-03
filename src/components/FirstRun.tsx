import { useCallback, useState, type ReactNode } from "react";
import { Glyph, Logo } from "../brand";
import { CANNOT_YET, EXAMPLE_GOALS } from "../firstRun/examples";
import { NewRecipe, type NewRecipeRequest } from "./NewRecipe";
import { PlumbingLine, plumbingState } from "./Plumbing";
import type { ModelsResponse, PlannerCandidate, PlannerPreview, RecipeInfo } from "../types";

/* The first run is a goal. One screen, seen once: the goal box with example goals under it, Draft it,
   and the plumbing (Ollama, a model that can plan) as one line at the bottom that unfolds only when
   something is missing. Browse and skip leave without drafting; all three mark the first run done. */

interface Props {
  recipes: RecipeInfo[];
  reserved?: string[];
  busy: boolean;
  error: string | null;
  progress?: string | null;
  problems?: string[] | null;
  onCreate: (req: NewRecipeRequest) => void;
  onStop?: () => void;
  below?: ReactNode;
  models: ModelsResponse | null;
  onRefreshModels: () => void;
  onOpenSettings: () => void;
  /** Save a settings patch; the screen marks the first run done with it when the person browses or skips. */
  onSave: (patch: Record<string, unknown>) => void;
  /** Leave for the Catalog. */
  onBrowse: () => void;
  /** Leave for the Library. */
  onSkip: () => void;
}

export function FirstRun({ recipes, reserved, busy, error, progress, problems, onCreate, onStop, below, models, onRefreshModels, onOpenSettings, onSave, onBrowse, onSkip }: Props) {
  const [planner, setPlanner] = useState<{ preview: PlannerPreview | null; chosen: PlannerCandidate | null }>({ preview: null, chosen: null });
  const [changing, setChanging] = useState(false);
  const onPlanner = useCallback((info: { preview: PlannerPreview | null; chosen: PlannerCandidate | null }) => setPlanner(info), []);
  const state = plumbingState(models);
  const blocked = state === "ready" || state === "loading" ? null : "needs a model that can plan";
  const browse = () => { onSave({ firstRun: false }); onBrowse(); };
  const skip = () => { onSave({ firstRun: false }); onSkip(); };

  return (
    <div className="first-run">
      <div className="first-run-head">
        <Logo size={48} />
        <p className="boot-line glow">Describe the goal. Get the result. See how it was made.</p>
      </div>
      <NewRecipe
        recipes={recipes} reserved={reserved} busy={busy} error={error} progress={progress} problems={problems}
        onCreate={onCreate} onCancel={skip} onStop={onStop} below={below}
        starts={["goal"]} examples={EXAMPLE_GOALS} blocked={blocked} planWith={changing} onPlanner={onPlanner}
        afterGoal={
          <details className="cannot-yet">
            <summary className="muted"><Glyph name="help" /> What Stillroom cannot do yet</summary>
            <p className="muted">{CANNOT_YET}</p>
          </details>
        }
        actions={
          <span className="first-run-links muted">
            or <button type="button" className="link" disabled={busy} onClick={browse}>browse the catalog</button>
            <span className="sep">·</span>
            <button type="button" className="link" disabled={busy} onClick={skip}>skip for now</button>
          </span>
        }
        foot={
          <PlumbingLine models={models} preview={planner.preview} chosen={planner.chosen} changing={changing}
            onRefreshModels={onRefreshModels} onOpenSettings={onOpenSettings} onChange={() => setChanging((v) => !v)} />
        }
      />
    </div>
  );
}
