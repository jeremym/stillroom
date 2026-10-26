export type Capability =
  | "text" | "structured_output" | "tool_calling" | "thinking" | "vision"
  | "embedding" | "text_to_speech" | "speech_to_text" | "image_generation";

export interface InstalledModel {
  id: string;
  provider: string;
  name: string;
  capabilities: Capability[];
  contextWindow: number | null;
  parameterSize: string | null;
}

export interface ProviderStatus {
  id: string;
  label: string;
  baseUrl: string;
  available: boolean;
}

export interface ModelsResponse {
  providers: ProviderStatus[];
  models: InstalledModel[];
}

export interface ModelSlot {
  requires: Capability[];
  preference?: string;
  recommended?: string;
  description?: string;
}

export interface Recipe {
  id: string;
  name: string;
  description?: string;
  inputs: Record<string, { type: string; label?: string; description?: string; accept?: string[] }>;
  modelSlots: Record<string, ModelSlot>;
  steps: Array<{ id: string; kind: string; name?: string }>;
  outputs: Record<string, { from: string; type: string; label?: string }>;
}

export type StepStatus = "pending" | "running" | "succeeded" | "failed" | "skipped";
export type RunStatus = "running" | "succeeded" | "failed" | "cancelled";

export interface StepRecord {
  runId: string;
  stepId: string;
  kind: string;
  name: string | null;
  position: number;
  status: StepStatus;
  startedAt: string | null;
  finishedAt: string | null;
  durationMs: number | null;
  model: string | null;
  promptPath: string | null;
  promptTokens: number | null;
  completionTokens: number | null;
  inputs: unknown;
  output: unknown;
  error: string | null;
}

export interface Run {
  id: string;
  recipeId: string;
  recipeName: string;
  status: RunStatus;
  startedAt: string;
  finishedAt: string | null;
  inputs: Record<string, unknown>;
  resolutions: Record<string, { slot: string; modelId: string; via: string }>;
  steps: StepRecord[];
  outputs: Record<string, unknown>;
  error: string | null;
}

export interface RunSummary {
  id: string;
  recipeId: string;
  recipeName: string;
  status: RunStatus;
  startedAt: string;
  finishedAt: string | null;
  stepCount: number;
  promptTokens: number;
  completionTokens: number;
}

export interface RuntimeStatus {
  schemaTitle: string;
  dbPath: string;
  tools: string[];
  runCount: number;
}
