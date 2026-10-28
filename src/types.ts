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
  /** True when the model runs on this Mac. */
  local: boolean;
}

export interface ProviderStatus {
  id: string;
  label: string;
  baseUrl: string;
  available: boolean;
  local: boolean;
}

/** The Codex CLI as found on this Mac, whether or not it is turned on as a provider. */
export interface CodexStatus {
  installed: boolean;
  path: string | null;
  version: string | null;
  signedIn: boolean;
  defaultModel: string | null;
}

export interface TryResult {
  ok: boolean;
  durationMs: number;
  answer: string | null;
  error: string | null;
  promptTokens: number | null;
  completionTokens: number | null;
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
export type RunStatus = "running" | "succeeded" | "failed" | "cancelled" | "needs_review";

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
  /** The output is one the user edited, not generated. */
  edited?: boolean;
  /** For steps inside a map: the map step's id. */
  parent?: string | null;
  /** Why a skipped step was skipped. */
  skipReason?: string | null;
  /** For agent steps: every tool call the model made. */
  toolCalls?: ToolCallRecord[];
}

export interface ToolCallRecord {
  tool: string;
  inputs: unknown;
  output: unknown;
  error: string | null;
  durationMs: number;
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
  /** Edits this run was asked to apply, by step id. */
  edits?: Record<string, { output: unknown; inputsFingerprint: string }>;
  /** What started the run, when it was not the Run button. */
  trigger?: RunTrigger | null;
  /** The run this one continued (an approval, an edit, a watch running again). */
  continues?: string | null;
}

export interface RunTrigger {
  kind: "watch" | string;
  watchId: string;
  folder: string;
  /** File names whose change set the run off; empty for "Run now". */
  changed: string[];
}

/** A folder watch: a recipe that runs by itself when something lands in a folder. */
export interface Watch {
  id: string;
  recipeId: string;
  recipeName: string;
  /** The recipe's folder input the watched folder fills. */
  input: string;
  folder: string;
  inputs: Record<string, unknown>;
  pins: Record<string, string>;
  enabled: boolean;
  settleMs: number;
  createdAt: string;
  lastFiredAt: string | null;
  lastRunId: string | null;
}

export type WatchPhase = "watching" | "running" | "queued" | "paused" | "failed";

export interface WatchView extends Watch {
  phase: WatchPhase;
  note: string | null;
}

export interface WatchRunEvent {
  watchId: string;
  recipeId: string;
  phase: "started" | "finished" | "failed";
  runId: string | null;
  status: RunStatus | null;
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
  trigger?: RunTrigger | null;
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
  dependsOn: string[];
  /** Recipe inputs this step reads (for a map, including its inner steps). */
  usesInputs: string[];
  /** The condition that must be set for the step to run, if any. */
  when?: string | null;
  /** For map steps: the inner steps. */
  steps?: StepInfo[];
  /** For map steps: the name each item is bound to. */
  itemName?: string | null;
  /** For agent steps: tools it may call. */
  allowedTools?: string[];
  /** For review steps: paths inside each item to display. */
  show?: string[];
  title?: string | null;
}

export interface Permissions {
  filesystem?: { read?: string[]; write?: string[] };
  network?: string[];
  cloudModels?: boolean;
  shell?: boolean;
  credentials?: string[];
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
  /** A glyph name for the card and the breadcrumb. */
  icon?: string | null;
  source: RecipeSource;
  path: string;
  inputs: Record<string, InputSpec>;
  modelSlots: Record<string, ModelSlot>;
  permissions?: Permissions | null;
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

/** A tool the runtime has, as shown to a recipe author. */
export interface ToolInfo {
  id: string;
  description: string;
  /** JSON Schema of the inputs object, when the tool describes itself. */
  parameters: { properties?: Record<string, { type?: string; description?: string }>; required?: string[] } | null;
  cacheable: boolean;
}

/** A recipe as it is on disk, for editing: recipe.yaml as JSON plus its prompt and schema files. */
export interface Draft {
  yaml: RecipeDoc;
  files: Record<string, string>;
}

export interface DraftResponse {
  id: string;
  source: RecipeSource;
  path: string;
  draft: Draft;
}

export interface SaveResponse {
  recipe: RecipeInfo;
  path: string;
  shadowsBuiltin: boolean;
}

/** One step as written in recipe.yaml. Loose on purpose: the schema is the source of truth. */
export interface StepDoc {
  id: string;
  kind: string;
  name?: string;
  description?: string;
  inputs?: Record<string, unknown>;
  when?: string;
  dependsOn?: string[];
  outputSchema?: unknown;
  checkpoint?: boolean;
  tool?: string;
  slot?: string;
  prompt?: string;
  params?: { temperature?: number; maxTokens?: number; think?: boolean | string; contextLength?: number };
  allowedTools?: string[];
  maxTurns?: number;
  maxToolCalls?: number;
  over?: string;
  as?: string;
  concurrency?: number;
  steps?: StepDoc[];
  items?: string;
  title?: string;
  show?: string[];
  [extra: string]: unknown;
}

export interface RecipeDoc {
  schemaVersion: string;
  id: string;
  name: string;
  version?: string;
  description?: string;
  icon?: string;
  inputs?: Record<string, InputSpec>;
  modelSlots?: Record<string, ModelSlot>;
  permissions?: Permissions;
  steps: StepDoc[];
  outputs: Record<string, OutputSpec>;
  [extra: string]: unknown;
}

/** Settings, as the Rust store holds them. Every key is optional; missing means the default. */
export interface Settings {
  look?: { theme?: "green" | "amber" | "paper"; size?: "normal" | "larger" | "largest"; scanlines?: boolean };
  ollama?: { baseUrl?: string };
  models?: {
    preference?: "accuracy" | "speed";
    byCapability?: Record<string, string>;
    /** Seconds one model call may take; 0 means no limit. Default 600. */
    timeoutSecs?: number;
    /** Context length for steps whose recipe says nothing. */
    contextLength?: number;
    /** Thinking for steps whose recipe says nothing: true, false, or unset (the model's own default). */
    think?: boolean | null;
  };
  /** The Codex CLI as a cloud provider: off unless turned on here. */
  codex?: { enabled?: boolean; path?: string; models?: string[] };
  /** Model pins per recipe: recipe id → slot → model id. */
  pins?: Record<string, Record<string, string>>;
  firstRun?: boolean;
}

export interface PullEvent {
  name: string;
  status: string;
  total: number | null;
  completed: number | null;
  done: boolean;
  error: string | null;
}

export interface ResolutionPreview {
  resolutions: Record<string, { slot: string; modelId: string; via: "pinned" | "default" | "recommended" | "capability" }>;
  problems: Record<string, string>;
}

export interface PermissionEntry {
  id: string;
  label: string;
  why: string;
  need: "required" | "optional" | "not_needed";
  status: "granted" | "denied" | "unknown" | "not_needed" | "in_use" | "not_in_use";
  pane: string | null;
}
