import { describe, expect, it } from "vitest";
import { nextPlace } from "./Dial";

const places = [{ id: "a", label: "A" }, { id: "b", label: "B" }, { id: "c", label: "C" }];

describe("nextPlace", () => {
  it("moves one step and stops at the ends", () => {
    expect(nextPlace(places, "a", 1)).toBe("b");
    expect(nextPlace(places, "b", -1)).toBe("a");
    expect(nextPlace(places, "a", -1)).toBe("a");
    expect(nextPlace(places, "c", 1)).toBe("c");
  });
  it("treats an unknown place as the first", () => {
    expect(nextPlace(places, "zzz", 1)).toBe("b");
  });
});
