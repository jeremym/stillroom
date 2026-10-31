import { describe, expect, it } from "vitest";
import { defaultInputs } from "./defaults";
import type { InputSpec } from "../types";

describe("defaultInputs", () => {
  it("turns declared defaults into field values and skips the rest", () => {
    const inputs: Record<string, InputSpec> = {
      urls: { type: "urls", default: "https://a.example\n https://b.example \n" },
      more: { type: "urls", default: ["https://c.example"] },
      notes: { type: "text", default: "remember this" },
      count: { type: "number", default: 3 },
      flag: { type: "boolean", default: true },
      file: { type: "file" },
      empty: { type: "text", default: "" },
    };
    expect(defaultInputs(inputs)).toEqual({
      urls: ["https://a.example", "https://b.example"],
      more: ["https://c.example"],
      notes: "remember this",
      count: 3,
      flag: true,
    });
  });
});
