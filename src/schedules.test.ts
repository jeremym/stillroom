import { describe, expect, it } from "vitest";
import { daysToSend, daysWord, nextWord, triggerWord, whenWord } from "./schedules";

describe("a schedule in words", () => {
  it("says the days plainly", () => {
    expect(daysWord([])).toBe("every day");
    expect(daysWord(["mon", "tue", "wed", "thu", "fri", "sat", "sun"])).toBe("every day");
    expect(daysWord(["mon", "tue", "wed", "thu", "fri"])).toBe("weekdays");
    expect(daysWord(["sun", "sat"])).toBe("weekends");
    expect(daysWord(["fri", "mon", "wed"])).toBe("Mon, Wed, Fri");
    expect(daysWord(["sun"])).toBe("Sun");
  });

  it("joins the days and the time", () => {
    expect(whenWord("07:30", [])).toBe("every day at 07:30");
    expect(whenWord("18:00", ["mon", "wed", "fri"])).toBe("Mon, Wed, Fri at 18:00");
  });

  it("sends every day as the empty list, otherwise the days in week order", () => {
    expect(daysToSend(new Set(["mon", "tue", "wed", "thu", "fri", "sat", "sun"]))).toEqual([]);
    expect(daysToSend(new Set(["fri", "mon"]))).toEqual(["mon", "fri"]);
    expect(daysToSend(new Set())).toEqual([]);
  });

  it("says when the next run is, relative to now", () => {
    const now = new Date(2026, 10, 2, 9, 0); // Mon 2 Nov 2026, 09:00 local
    const at = (y: number, m: number, d: number, h: number, min: number) => new Date(y, m, d, h, min).toISOString();
    expect(nextWord(null, "07:30", now)).toBeNull();
    expect(nextWord("not a date", "07:30", now)).toBeNull();
    expect(nextWord(at(2026, 10, 2, 9, 12), "09:12", now)).toBe("next in 12 min");
    expect(nextWord(at(2026, 10, 2, 11, 0), "11:00", now)).toBe("next in 2 h");
    expect(nextWord(at(2026, 10, 2, 18, 0), "18:00", now)).toBe("next today at 18:00");
    expect(nextWord(at(2026, 10, 3, 7, 30), "07:30", now)).toBe("next tomorrow at 07:30");
    expect(nextWord(at(2026, 10, 4, 7, 30), "07:30", now)).toBe("next Wed at 07:30");
    expect(nextWord(at(2026, 10, 8, 7, 30), "07:30", now)).toBe("next Sun at 07:30");
    expect(nextWord(at(2026, 10, 9, 7, 30), "07:30", now)).toMatch(/^next .*Nov.* at 07:30$/);
    expect(nextWord(at(2026, 10, 2, 8, 0), "08:00", now)).toBe("next any moment now");
  });

  it("names what started a run", () => {
    expect(triggerWord(null)).toBeNull();
    expect(triggerWord(undefined)).toBeNull();
    expect(triggerWord({ kind: "watch", watchId: "w", folder: "/x", changed: [] })).toBe("by a watch");
    expect(triggerWord({ kind: "schedule", scheduleId: "s", time: "07:30" })).toBe("by a schedule");
  });
});
