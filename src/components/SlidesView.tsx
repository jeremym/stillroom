import { useEffect, useRef, useState } from "react";

export interface Slide {
  title: string;
  bullets: string[];
  notes: string;
  concept: string;
}
export interface Deck {
  title: string;
  subtitle?: string;
  slides: Slide[];
}

export function isDeck(v: unknown): v is Deck {
  return typeof v === "object" && v !== null && Array.isArray((v as { slides?: unknown }).slides)
    && typeof (v as { title?: unknown }).title === "string";
}

export function SlidesView({ deck }: { deck: Deck }) {
  const [i, setI] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  const n = deck.slides.length;
  const slide = deck.slides[i];

  useEffect(() => setI(0), [deck]);

  function onKey(e: React.KeyboardEvent) {
    if (e.key === "ArrowRight" || e.key === " ") { setI((x) => Math.min(n - 1, x + 1)); e.preventDefault(); }
    if (e.key === "ArrowLeft") { setI((x) => Math.max(0, x - 1)); e.preventDefault(); }
  }

  if (!slide) return <p className="muted">Empty deck.</p>;
  return (
    <div className="deck" ref={ref} tabIndex={0} onKeyDown={onKey}>
      <ol className="deck-list">
        {deck.slides.map((s, k) => (
          <li key={k}>
            <button className={`link ${k === i ? "current" : ""}`} onClick={() => setI(k)}>{k + 1}. {s.title}</button>
          </li>
        ))}
      </ol>
      <div className="deck-main">
        <div className="slide">
          {i === 0 && <div className="slide-kicker">{deck.title}{deck.subtitle ? ` · ${deck.subtitle}` : ""}</div>}
          <h3 className="slide-title">{slide.title}</h3>
          <ul className="slide-bullets">
            {slide.bullets.map((b, k) => <li key={k}>{b}</li>)}
          </ul>
          <div className="slide-foot muted mono">{i + 1} / {n}{slide.concept ? ` · ${slide.concept}` : ""}</div>
        </div>
        <div className="row space">
          <button onClick={() => setI((x) => Math.max(0, x - 1))} disabled={i === 0}>← Previous</button>
          <span className="muted">arrow keys work when the deck is focused</span>
          <button onClick={() => setI((x) => Math.min(n - 1, x + 1))} disabled={i === n - 1}>Next →</button>
        </div>
        <p className="notes"><strong>Speaker notes.</strong> {slide.notes}</p>
      </div>
    </div>
  );
}

function esc(s: string) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/// A self-contained HTML deck: one section per slide, printable, no scripts.
export function deckToHtml(deck: Deck): string {
  const slides = deck.slides
    .map(
      (s, i) => `<section class="slide">
  <h2>${esc(s.title)}</h2>
  <ul>${s.bullets.map((b) => `<li>${esc(b)}</li>`).join("")}</ul>
  <aside class="notes"><strong>Notes.</strong> ${esc(s.notes)}</aside>
  <footer>${i + 1} / ${deck.slides.length}</footer>
</section>`,
    )
    .join("\n");
  return `<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(deck.title)}</title>
<style>
  body { margin: 0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #f3f2ee; color: #1c1c1e; }
  header { padding: 40px 48px 8px; }
  header h1 { margin: 0; font-size: 2rem; }
  header p { margin: 4px 0 0; color: #666; }
  .slide { background: #fff; margin: 24px 48px; padding: 40px 48px 56px; border-radius: 16px; box-shadow: 0 2px 12px rgba(0,0,0,.08); position: relative; page-break-after: always; }
  .slide h2 { margin: 0 0 16px; font-size: 1.8rem; }
  .slide ul { font-size: 1.25rem; line-height: 1.6; padding-left: 28px; margin: 0; }
  .notes { margin-top: 24px; padding-top: 16px; border-top: 1px solid #e4e4e0; color: #555; font-size: 0.95rem; }
  footer { position: absolute; right: 24px; bottom: 16px; color: #999; font-size: 0.8rem; }
  @media print { body { background: #fff; } .slide { box-shadow: none; margin: 0; border-radius: 0; } }
</style></head>
<body>
<header><h1>${esc(deck.title)}</h1>${deck.subtitle ? `<p>${esc(deck.subtitle)}</p>` : ""}</header>
${slides}
</body></html>
`;
}

/// Build a .pptx with pptxgenjs: title slide, one slide per entry, speaker notes attached.
export async function deckToPptx(deck: Deck): Promise<Uint8Array> {
  const { default: PptxGenJS } = await import("pptxgenjs");
  const pptx = new PptxGenJS();
  pptx.layout = "LAYOUT_16x9";
  pptx.title = deck.title;

  const title = pptx.addSlide();
  title.background = { color: "F3F2EE" };
  title.addText(deck.title, { x: 0.6, y: 1.6, w: 8.8, h: 1.2, fontSize: 36, bold: true, color: "1C1C1E", fontFace: "Helvetica" });
  if (deck.subtitle) {
    title.addText(deck.subtitle, { x: 0.6, y: 2.9, w: 8.8, h: 0.8, fontSize: 18, color: "666666", fontFace: "Helvetica" });
  }

  deck.slides.forEach((s, i) => {
    const slide = pptx.addSlide();
    slide.addText(s.title, { x: 0.6, y: 0.4, w: 8.8, h: 0.9, fontSize: 28, bold: true, color: "1C1C1E", fontFace: "Helvetica" });
    slide.addText(
      s.bullets.map((b) => ({ text: b, options: { bullet: true, breakLine: true } })),
      { x: 0.6, y: 1.5, w: 8.8, h: 3.4, fontSize: 18, color: "1C1C1E", fontFace: "Helvetica", valign: "top" },
    );
    slide.addText(`${i + 1} / ${deck.slides.length}`, { x: 8.6, y: 5.1, w: 1, h: 0.3, fontSize: 10, color: "999999", align: "right" });
    if (s.notes) slide.addNotes(s.notes);
  });

  const out = await pptx.write({ outputType: "uint8array" });
  return out as Uint8Array;
}
