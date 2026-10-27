import { Glyph } from "./Glyph";

/* The alembic (a still) and the wordmark. The mark alone is the app icon. Sizes are inline so two
   logos on one page (the Rail and a boot screen) do not fight over a shared style. */
export function Logo({ size = 36, word = true }: { size?: number; word?: boolean }) {
  return (
    <span className="logo" style={{ display: "inline-flex", alignItems: "center", gap: Math.round(size / 3), ["--logo-size" as string]: `${size}px` }}>
      <Glyph name="alembic" className="glow-svg logo-mark" title="Stillroom" />
      {word && <span className="wordmark glow" style={{ fontFamily: "var(--display)", fontSize: Math.round(size * 0.85), letterSpacing: "0.22em", textTransform: "uppercase", lineHeight: 1 }}>Stillroom</span>}
    </span>
  );
}
