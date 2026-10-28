/* Where a model runs, from its provider-qualified id. Ollama is the only local provider;
   anything else leaves this Mac and is marked amber wherever the model is named. */

export const LOCAL_PROVIDERS = ["ollama"];

export function providerOf(modelId: string): string {
  const i = modelId.indexOf(":");
  return i === -1 ? modelId : modelId.slice(0, i);
}

/** The bare model name, without the provider prefix. */
export function shortModel(modelId: string): string {
  const i = modelId.indexOf(":");
  return i === -1 ? modelId : modelId.slice(i + 1);
}

export function isLocalModel(modelId: string | null | undefined): boolean {
  if (!modelId) return true;
  return LOCAL_PROVIDERS.includes(providerOf(modelId));
}

const PROVIDER_WORD: Record<string, string> = { ollama: "local", codex: "Codex · cloud" };

/** The word for the tag beside a model: "local", or the cloud provider's name with "cloud". */
export function whereWord(modelId: string | null | undefined): string {
  if (!modelId) return "local";
  return PROVIDER_WORD[providerOf(modelId)] ?? `${providerOf(modelId)} · cloud`;
}

/** Ollama model names people usually start with; the first one not yet installed is suggested. */
export const STARTER_MODELS = ["qwen3:8b", "gemma3:12b", "llama3.2:3b", "qwen3:4b"];

export function suggestPull(installed: string[]): string {
  const have = new Set(installed.map((n) => n.toLowerCase()));
  return STARTER_MODELS.find((n) => !have.has(n)) ?? STARTER_MODELS[0];
}

/** True when `name` (as typed) is already installed, with or without the `:latest` suffix. */
export function isInstalled(name: string, installed: string[]): boolean {
  const n = name.trim().toLowerCase();
  if (!n) return false;
  const have = new Set(installed.map((x) => x.toLowerCase()));
  return have.has(n) || have.has(`${n}:latest`) || (n.endsWith(":latest") && have.has(n.slice(0, -7)));
}
