import { useState } from "react";
import { api } from "../api";
import { fixIds, notesOf, problemsOf } from "./findings";
import type { Draft, Finding } from "../types";

/* The checker's findings above the recipe map while editing. Problems (in red) must be fixed before
   the draft can be saved. Notes (quiet) are worth a look and never block Save; a note the engine knows
   how to fix carries a button, and the rewritten draft goes back through the editor like any edit.
   Without a draft to fix (the import's preview) the findings are read only, and the problems' heading
   says what they stop. */

interface Props {
  findings: Finding[];
  /** The draft the findings are about; what a fix is applied to. Left out, nothing can be fixed here. */
  draft?: Draft;
  /** The rewritten draft, to be taken in like an edit (dirty state and all). */
  onDraft?: (draft: Draft) => void;
  /** The heading over the problems; what they stand in the way of. */
  blocks?: string;
}

export function FindingsPanel({ findings, draft, onDraft, blocks = "Must be fixed before saving" }: Props) {
  const [fixing, setFixing] = useState(false);
  const [fixError, setFixError] = useState<string | null>(null);
  const problems = problemsOf(findings);
  const notes = notesOf(findings);
  const canFix = Boolean(draft && onDraft);
  const fixable = canFix ? fixIds(notes) : [];

  async function fix(ids: string[]) {
    if (!draft || !onDraft) return;
    setFixing(true);
    setFixError(null);
    try {
      onDraft(await api.fixRecipeDraft(draft, ids));
    } catch (e) {
      setFixError(String(e));
    } finally {
      setFixing(false);
    }
  }

  if (problems.length === 0 && notes.length === 0 && !fixError) return null;
  return (
    <div className="tty flow-problems" role="log" aria-live="polite">
      {problems.length > 0 && (
        <>
          <div className="head">{blocks}</div>
          {problems.map((p, i) => <div key={`p${i}`} className="red"><span className="mark">✗ </span>{p.text}</div>)}
        </>
      )}
      {notes.length > 0 && (
        <>
          <div className="head">
            Worth a look
            {fixable.length > 1 && <> · <button className="link" disabled={fixing} onClick={() => fix(fixable)}>Fix all</button></>}
          </div>
          {notes.map((n, i) => (
            <div key={`n${i}`} className="dim finding-note">
              <span className="mark">· </span>{n.text}
              {n.fix && canFix && <> <button className="link" disabled={fixing} onClick={() => fix([n.fix as string])}>{n.fixLabel || "Fix it"}</button></>}
            </div>
          ))}
        </>
      )}
      {fixError && <div className="red"><span className="mark">✗ </span>The fix did not apply: {fixError}</div>}
    </div>
  );
}
