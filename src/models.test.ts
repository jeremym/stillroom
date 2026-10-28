import { describe, expect, it } from "vitest";
import { isInstalled, isLocalModel, rememberProviders, shortModel, suggestPull, whereWord } from "./models";

describe("models", () => {
  it("knows local from cloud by provider", () => {
    expect(isLocalModel("ollama:qwen3:8b")).toBe(true);
    expect(isLocalModel("codex:gpt-5.6-sol")).toBe(false);
    expect(isLocalModel(null)).toBe(true);
    expect(whereWord("ollama:qwen3:8b")).toBe("local");
    expect(whereWord("codex:gpt-5.6-sol")).toBe("Codex · cloud");
    expect(shortModel("ollama:qwen3:8b")).toBe("qwen3:8b");
    rememberProviders([{ id: "openai", local: true }]);
    expect(whereWord("openai:qwen3")).toBe("OpenAI · local");
    rememberProviders([{ id: "openai", local: false }]);
    expect(whereWord("openai:gpt-5")).toBe("OpenAI · cloud");
  });
  it("suggests a model that is not installed yet", () => {
    expect(suggestPull([])).toBe("qwen3:8b");
    expect(suggestPull(["qwen3:8b"])).toBe("gemma3:12b");
    expect(isInstalled("qwen3:8b", ["qwen3:8b"])).toBe(true);
    expect(isInstalled("qwen3", ["qwen3:latest"])).toBe(true);
    expect(isInstalled("gemma3:12b", ["qwen3:8b"])).toBe(false);
  });
});
