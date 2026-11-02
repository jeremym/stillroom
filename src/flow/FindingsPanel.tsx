import { useState } from "react";
import { api } from "../api";
import { fixIds, notesOf, problemsOf } from "./findings";
import type { Draft, Finding } from "../types";

/* The checker's findings above the recipe map while editing. Problems (in red) must be fixed before
   the draft can be saved. Notes (quiet) are worth a look and never block Save; a note the engine knows
   how to fix carries a button, and the rewritten draft goes back through the editor like any edit. */

interface Props {
  findings: Finding[];
  /** The draft the findings are about; what a fix is applied to. */
  draft: Draft;
  /** The rewritten draft, to be taken in like an edit (dirty state and all). */
  onDraft: (draft: Draft) => void;
}

export function FindingsPanel({ findings, draft, onDraft }: Props) {
  const [fixing, setFixing] = useState(false);
  const [fixError, setFixError] = useState<string | null>(null);
  const problems = problemsOf(findings);
  const notes = notesOf(findings);
  const fixable = fixIds(notes);

  async function fix(ids: string[]) {
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
          <div className="head">Must be fixed before saving</div>
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
              {n.fix && <> <button className="link" disabled={fixing} onClick={() => fix([n.fix as string])}>{n.fixLabel || "Fix it"}</button></>}
            </div>
          ))}
        </>
      )}
      {fixError && <div className="red"><span className="mark">✗ </span>The fix did not apply: {fixError}</div>}
    </div>
  );
}
