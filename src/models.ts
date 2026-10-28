/* Where a model runs, from its provider-qualified id. Ollama is the only local provider;
   anything else leaves this Mac and is marked amber wherever the model is named. */

/* Which providers are local is decided by the Rust side (an OpenAI-compatible server on
   localhost counts as local). The registry starts with Ollama and learns the rest when a
   model list arrives. */
const localProviders = new Set<string>(["ollama"]);
const cloudProviders = new Set<string>();

export function rememberProviders(providers: { id: string; local: boolean }[]): void {
  for (const p of providers) {
    if (p.local) { localProviders.add(p.id); cloudProviders.delete(p.id); } else { cloudProviders.add(p.id); localProviders.delete(p.id); }
  }
}

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
  return localProviders.has(providerOf(modelId));
}

const PROVIDER_NAME: Record<string, string> = { ollama: "Ollama", codex: "Codex", openai: "OpenAI" };

/** The word for the tag beside a model: "local", or the cloud provider's name with "cloud". */
export function whereWord(modelId: string | null | undefined): string {
  if (!modelId) return "local";
  const p = providerOf(modelId);
  if (isLocalModel(modelId)) return p === "ollama" ? "local" : `${PROVIDER_NAME[p] ?? p} · local`;
  return `${PROVIDER_NAME[p] ?? p} · cloud`;
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
