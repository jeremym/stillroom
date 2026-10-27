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
  reusedFrom?: string | null;
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
  recipeRoots: string[];
}

export type RecipeSource = "builtin" | "user";

export interface InputSpec {
  type: "string" | "text" | "number" | "boolean" | "file" | "files" | "url" | "urls" | "directory" | "json";
  label?: string;
  description?: string;
  required?: boolean;
  default?: unknown;
  accept?: string[];
}

export interface StepInfo {
  id: string;
  kind: string;
  name: string | null;
  slot: string | null;
  tool: string | null;
}

export interface OutputSpec {
  from: string;
  type: string;
  label?: string;
  description?: string;
}

export interface RecipeInfo {
  id: string;
  name: string;
  version: string | null;
  description: string | null;
  source: RecipeSource;
  path: string;
  inputs: Record<string, InputSpec>;
  modelSlots: Record<string, ModelSlot>;
  steps: StepInfo[];
  outputs: Record<string, OutputSpec>;
}

export interface RecipesResponse {
  recipes: RecipeInfo[];
  errors: { path: string; error: string }[];
}

export interface ModelUsage {
  modelId: string;
  calls: number;
  promptTokens: number;
  completionTokens: number;
  totalMs: number;
}

export interface RunEstimate {
  basedOnRuns: number;
  avgPromptTokens: number;
  avgCompletionTokens: number;
  avgDurationMs: number;
}
