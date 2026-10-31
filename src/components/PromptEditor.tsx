import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { Glyph } from "../brand";
import type { PromptState } from "../types";

/* One of the engine's own prompts (the planner): read it, change it, go back to any earlier
   revision or the original. Saving never overwrites; it adds a revision and uses it. */

interface Props {
  name: string;
  /** Fewer rows and no title, for a disclosure inside another screen. */
  compact?: boolean;
  onError?: (message: string) => void;
}

function when(iso: string | null) {
  if (!iso) return "";
  const d = new Date(iso);
  return isNaN(d.getTime()) ? "" : d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

export function PromptEditor({ name, compact = false, onError }: Props) {
  const [state, setState] = useState<PromptState | null>(null);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const fail = useCallback((e: unknown) => { onError?.(String(e)); }, [onError]);

  const load = useCallback(() => {
    api.readSystemPrompt(name).then((s) => { setState(s); setText(s.text); }).catch(fail);
  }, [name, fail]);
  useEffect(() => { load(); }, [load]);
  useEffect(() => {
    const un = listen<string>("prompts:changed", (ev) => { if (ev.payload === name) load(); });
    return () => { un.then((f) => f()); };
  }, [name, load]);

  if (!state) return <p className="muted">…</p>;
  const dirty = text.trim() !== state.text.trim();
  const isOriginal = text.trim() === state.original.trim();
  const usingWord = state.current === 0 ? "the original" : `revision ${state.current} of ${state.versions.length - 1}`;

  async function act(f: () => Promise<PromptState>, said: string) {
    setBusy(true);
    setNote(null);
    try {
      const s = await f();
      setState(s);
      setText(s.text);
      setNote(said);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="prompt-editor">
      {!compact && <p className="muted">{state.purpose}</p>}
      <div className="row prompt-bar" style={{ marginTop: 0 }}>
        <label>
          <span className="muted">Using</span>{" "}
          <select value={state.current} disabled={busy} aria-label="Revision in use"
            onChange={(e) => act(() => api.useSystemPromptVersion(name, Number(e.target.value)), Number(e.target.value) === 0 ? "Back to the original." : `Using revision ${e.target.value}.`)}>
            {state.versions.map((v) => (
              <option key={v.n} value={v.n}>{v.n === 0 ? "Original (as shipped)" : `Revision ${v.n}${v.savedAt ? ` · ${when(v.savedAt)}` : ""}`}</option>
            ))}
          </select>
        </label>
        <span className="muted">{dirty ? "Unsaved changes" : `Showing ${usingWord}.`}</span>
      </div>
      <textarea
        className="mono"
        rows={compact ? 14 : 28}
        value={text}
        spellCheck={false}
        disabled={busy}
        aria-label={`${state.title} prompt`}
        onChange={(e) => setText(e.target.value)}
      />
      <div className="row" style={{ marginTop: 6 }}>
        <button className="primary" style={{ marginTop: 0 }} disabled={busy || !dirty}
          onClick={() => act(() => api.saveSystemPrompt(name, text), isOriginal ? "That is the original text, so it is in use again." : "Saved as a new revision and in use.")}>
          {busy ? "Saving…" : "Save as a new revision"}
        </button>
        <button disabled={busy || !dirty} onClick={() => { setText(state.text); setNote(null); }}>Discard changes</button>
        <button disabled={busy || (state.current === 0 && !dirty)} title="Point back at the text the app shipped with. Your revisions stay in the list."
          onClick={() => act(() => api.useSystemPromptVersion(name, 0), "Back to the original. Your revisions are still in the list.")}>
          <Glyph name="recipe" /> Reset to original
        </button>
      </div>
      {note && <p className="muted">{note}</p>}
    </div>
  );
}
