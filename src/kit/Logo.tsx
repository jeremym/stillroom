import { Glyph } from "./Glyph";

/* The alembic (a still) and the wordmark. The mark alone is the app icon. */
export function Logo({ size = 36, word = true }: { size?: number; word?: boolean }) {
  return (
    <span className="logo" style={{ display: "inline-flex", alignItems: "center", gap: 12 }}>
      <Glyph name="alembic" className="glow-svg" title="Stillroom" />
      {word && <span className="wordmark glow">Stillroom</span>}
      <style>{`.logo .glyph{width:${size}px;height:${size}px}.wordmark{font-family:var(--display);font-size:${Math.round(size * 0.85)}px;letter-spacing:.22em;text-transform:uppercase;line-height:1}`}</style>
    </span>
  );
}
