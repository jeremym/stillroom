import type { UpdateCheck } from "./types";

/** When a check happened, in words a person reads at a glance: "just now", "3 hours ago", "yesterday", a date. */
export function agoWord(iso: string, now: Date = new Date()): string {
  const t = new Date(iso).getTime();
  if (isNaN(t)) return "";
  const mins = Math.max(0, Math.round((now.getTime() - t) / 60_000));
  if (mins < 2) return "just now";
  if (mins < 60) return `${mins} min ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.round(hours / 24);
  if (days === 1) return "yesterday";
  if (days < 14) return `${days} days ago`;
  return new Date(iso).toLocaleDateString();
}

/** The one line Settings › About says about the last check, and whether it is good news. */
export function updateWord(last: UpdateCheck | undefined, now: Date = new Date()): { text: string; tone: "muted" | "green" | "amber" } {
  if (!last) return { text: "Not checked yet.", tone: "muted" };
  const when = agoWord(last.checkedAt, now);
  if (last.error) return { text: `Could not check ${when}: ${last.error}.`, tone: "amber" };
  if (last.newer && last.latest) return { text: `Stillroom ${last.latest} is out; you have ${last.current}.`, tone: "green" };
  if (last.latest === last.current) return { text: `Up to date (${last.current}), checked ${when}.`, tone: "muted" };
  if (last.latest) return { text: `You have ${last.current}, ahead of the newest release (${last.latest}); checked ${when}.`, tone: "muted" };
  return { text: `Checked ${when}.`, tone: "muted" };
}
