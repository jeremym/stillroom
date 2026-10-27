/* Theme, text size, and scanlines live on <html> as data attributes and are remembered per machine. */
export type Theme = "green" | "amber" | "paper";
export type TextSize = "normal" | "larger" | "largest";

export interface Look { theme: Theme; size: TextSize; scanlines: boolean }
const KEY = "p1.look";
const DEFAULT: Look = { theme: "green", size: "normal", scanlines: false };

export function loadLook(): Look {
  try {
    return { ...DEFAULT, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
  } catch {
    return DEFAULT;
  }
}

export function applyLook(look: Look) {
  const root = document.documentElement;
  root.dataset.theme = look.theme;
  root.dataset.size = look.size;
  root.dataset.scanlines = look.scanlines ? "on" : "off";
  root.style.colorScheme = look.theme === "paper" ? "light" : "dark";
  try {
    localStorage.setItem(KEY, JSON.stringify(look));
  } catch { /* per-machine convenience only */ }
}
