import type { RunTrigger } from "./types";

/* Schedules in words. A schedule is a recipe that runs at a time of day, on the days chosen, while
   Stillroom is open. The engine keeps the time as "07:30" (24-hour, local) and the days as
   "mon".."sun", empty meaning every day; this file turns those into the lines the panels show. */

export const ALL_DAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;
export type Day = (typeof ALL_DAYS)[number];

export const DAY_LABEL: Record<Day, string> = { mon: "Mon", tue: "Tue", wed: "Wed", thu: "Thu", fri: "Fri", sat: "Sat", sun: "Sun" };

/** "every day", "weekdays", "weekends", or "Mon, Wed, Fri" in week order. */
export function daysWord(days: readonly string[]): string {
  const set = new Set(days);
  const picked = ALL_DAYS.filter((d) => set.has(d));
  if (picked.length === 0 || picked.length === 7) return "every day";
  const key = picked.join(",");
  if (key === "mon,tue,wed,thu,fri") return "weekdays";
  if (key === "sat,sun") return "weekends";
  return picked.map((d) => DAY_LABEL[d]).join(", ");
}

/** "every day at 07:30", "weekdays at 07:30", "Mon, Wed, Fri at 07:30". */
export function whenWord(time: string, days: readonly string[]): string {
  return `${daysWord(days)} at ${time}`;
}

/** The days to send the engine: every day is the empty list; otherwise the picked days in week order. */
export function daysToSend(picked: ReadonlySet<string>): string[] {
  const days = ALL_DAYS.filter((d) => picked.has(d));
  return days.length === 7 ? [] : days;
}

const HOUR = 60 * 60 * 1000;

/** The next firing in words, relative to `now`: "next in 12 min", "next in 2 h", "next today at 07:30",
 *  "next tomorrow at 07:30", "next Wed at 07:30". Null when there is no next time (the schedule is off). */
export function nextWord(nextAt: string | null, time: string, now: Date = new Date()): string | null {
  if (!nextAt) return null;
  const next = new Date(nextAt);
  if (Number.isNaN(next.getTime())) return null;
  const diff = next.getTime() - now.getTime();
  if (diff <= 0) return "next any moment now";
  if (diff < HOUR) return `next in ${Math.max(1, Math.round(diff / 60000))} min`;
  if (diff < 6 * HOUR) return `next in ${Math.round(diff / HOUR)} h`;
  const dayStart = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const daysAway = Math.round((dayStart(next) - dayStart(now)) / (24 * HOUR));
  if (daysAway === 0) return `next today at ${time}`;
  if (daysAway === 1) return `next tomorrow at ${time}`;
  if (daysAway < 7) return `next ${DAY_LABEL[ALL_DAYS[(next.getDay() + 6) % 7]]} at ${time}`;
  return `next ${next.toLocaleDateString(undefined, { day: "numeric", month: "short" })} at ${time}`;
}

/** "by a watch" or "by a schedule": the word history puts beside a run that did not start from the Run button. */
export function triggerWord(trigger: RunTrigger | null | undefined): string | null {
  if (!trigger) return null;
  return trigger.kind === "schedule" ? "by a schedule" : "by a watch";
}

/** The default a new schedule form opens with. */
export const DEFAULT_TIME = "07:30";
