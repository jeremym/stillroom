import { ModelsPanel } from "./ModelsPanel";
import { Zone, type Look } from "../kit";
import type { ModelsResponse, RuntimeStatus } from "../types";

/* Settings (⌘,): how the app looks, what it thinks with, and where things live. Grows into its own
   window in the next phase; the zones stay the same. */
interface Props {
  look: Look;
  onLook: (look: Look) => void;
  models: ModelsResponse | null;
  onRefreshModels: () => void;
  status: RuntimeStatus | null;
  onRevealRecipes: () => void;
}

export function SettingsPanel({ look, onLook, models, onRefreshModels, status, onRevealRecipes }: Props) {
  return (
    <>
      <Zone title="Look" sub="theme, text size">
        <div className="settings-row">
          <span className="label">Theme</span>
          <span className="segmented" role="group" aria-label="Theme">
            {(["green", "amber", "paper"] as const).map((t) => (
              <button key={t} aria-pressed={look.theme === t} onClick={() => onLook({ ...look, theme: t })}>{t}</button>
            ))}
          </span>
          <span className="label">Text size</span>
          <span className="segmented" role="group" aria-label="Text size">
            {(["normal", "larger", "largest"] as const).map((s) => (
              <button key={s} aria-pressed={look.size === s} onClick={() => onLook({ ...look, size: s })}>{s}</button>
            ))}
          </span>
          <span className="label">Scanlines</span>
          <span>
            <button className="toggle" aria-pressed={look.scanlines} onClick={() => onLook({ ...look, scanlines: !look.scanlines })}>
              <span className={look.scanlines ? "on" : ""}>on</span><span className={look.scanlines ? "" : "on"}>off</span>
            </button>
            <span className="muted"> · faint lines over the screen, off unless you like them</span>
          </span>
        </div>
      </Zone>
      <ModelsPanel models={models} onRefresh={onRefreshModels} />
      <Zone title="Cloud providers" sub="off">
        <p className="muted">Stillroom runs everything on this Mac. Cloud models are not set up and nothing leaves the machine. When they arrive they will be opt-in here, and every step that uses one will be marked in amber on the map.</p>
      </Zone>
      <Zone title="Folders" sub="where your recipes and results live">
        {status ? (
          <>
            {status.recipeRoots.map((r, i) => (
              <p key={r}><span className="status-word muted">{i === 0 ? "Built-in recipes" : "Your recipes"}</span><br /><span className="mono">{r}</span></p>
            ))}
            <button onClick={onRevealRecipes}>Show my recipes folder</button>
          </>
        ) : <p className="muted">…</p>}
      </Zone>
      <Zone title="About" sub="Stillroom 0.1, pre-alpha">
        <p className="muted">{status ? `${status.schemaTitle} v0 · ${status.runCount} runs · tools: ${status.tools.join(", ")}` : "…"}</p>
        <p className="muted">Keyboard: ⌘0 Library · ⌘1 Recipe · ⌘2 Set up · ⌘3 Running · ⌘4 Results · ⌘5 Open the door · ⌘N New recipe · ⌘R Run · ⌘, Settings · ⌘/ Shortcuts</p>
      </Zone>
    </>
  );
}
