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
  /** How fast the model wrote, in tokens per second, when the provider said (Ollama does). */
  completionTokensPerSecond?: number | null;
}

/** What one model managed last time it answered on this Mac. */
export interface ModelSpeed {
  tokensPerSecond: number;
  /** When, RFC 3339. */
  at: string;
}

export interface ModelsResponse {
  providers: ProviderStatus[];
  models: InstalledModel[];
  /** By model id; a model that has not answered yet is absent. */
  speeds?: Record<string, ModelSpeed>;
}

/** This Mac: what the models run on. Anything that could not be read is null. */
export interface Hardware {
  /** The chip, e.g. "Apple M3 Max". */
  chip: string | null;
  /** Physical memory, shared by the CPU and GPU on Apple silicon. */
  memoryBytes: number | null;
  gpuCores: number | null;
  cpuCores: number | null;
  /** The most a model's weights may take and still run comfortably (60 % of memory). */
  modelCeilingBytes: number | null;
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
  /** While a model step runs: where its call is. Cleared when the call ends. */
  phase?: CallPhase | null;
  /** Words in the prompt the model was sent. */
  promptWords?: number | null;
  /** Words written so far, then the total. */
  wordsSoFar?: number | null;
  /** Words per second while writing. */
  wordsPerSecond?: number | null;
  /** Milliseconds the provider spent loading the model and reading the prompt, when it says. */
  loadMs?: number | null;
  promptMs?: number | null;
}

/** Where a model call is: loading the model into memory, reading the prompt (prefill), or writing. */
export type CallPhase = "loading" | "reading" | "writing";

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

/** What started a run when it was not the Run button: a folder watch or a schedule. */
export type RunTrigger = WatchTrigger | ScheduleTrigger;

export interface WatchTrigger {
  kind: "watch";
  watchId: string;
  folder: string;
  /** File names whose change set the run off; empty for "Run now". */
  changed: string[];
}

export interface ScheduleTrigger {
  kind: "schedule";
  scheduleId: string;
  /** The time of day the schedule is set for, "07:30" (24-hour, local). */
  time: string;
}

/** A schedule: a recipe that runs by itself at a time of day, on the days chosen, while Stillroom is open. */
export interface Schedule {
  id: string;
  recipeId: string;
  recipeName: string;
  inputs: Record<string, unknown>;
  pins: Record<string, string>;
  /** "07:30", 24-hour, local time. */
  time: string;
  /** "mon".."sun"; empty means every day. */
  days: string[];
  enabled: boolean;
  createdAt: string;
  lastFiredAt: string | null;
  lastRunId: string | null;
  /** When it fires next, or null while off. */
  nextAt: string | null;
}

export interface ScheduleRunEvent {
  scheduleId: string;
  recipeId: string;
  phase: "started" | "finished" | "failed";
  runId: string | null;
  status: RunStatus | null;
  error: string | null;
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
  /** The version of Stillroom that is running, as the bundle says. */
  version: string;
  schemaTitle: string;
  dbPath: string;
  tools: string[];
  runCount: number;
  recipeRoots: string[];
}

/** Where a recipe came from: written here (built-in), contributed through the repository and reviewed
 *  before it shipped (community), one of the sample recipes that need the sample folders, or yours. */
export type RecipeSource = "builtin" | "community" | "sample" | "user";

/** Who wrote a recipe, as its recipe.yaml says. */
export interface RecipeAuthor {
  name: string;
  url?: string | null;
}

/** A duplicate's provenance: the catalog recipe it was made from and its version then. `newer` is
 *  true when the catalog has moved on and the copy's steps were never edited. */
export interface RecipeOrigin {
  catalog: string;
  version: string;
  newer: boolean;
}

/** Where a recipe sits for the user: on the shelf (the Library's Recipes zone), in the archive (taken
 *  off the shelf, Restore puts it back), or in the catalog (a built-in that is neither). A recipe of
 *  yours is never "catalog". */
export type ShelfState = "shelf" | "archive" | "catalog";

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

/** One row of what a run would touch (Set up › Permissions), checked against the inputs typed so far. */
export interface PermissionRow {
  kind: "read" | "write" | "reach" | "run";
  label: string;
  detail: string | null;
  status: "ok" | "not_chosen" | "cannot_read" | "cannot_write" | "missing";
  note: string | null;
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

/** One literal permission entry that needs the user's tick before the recipe runs: what kind of access,
 *  the entry as written in recipe.yaml, and the sentence to show for it. `grantedAt` comes back on a
 *  granted entry when the engine remembers when. */
export interface Grant {
  kind: GrantKind;
  entry: string;
  text: string;
  grantedAt?: string | null;
}

export type GrantKind = "read" | "write" | "network" | "shell" | "cloudModels" | "credentials";

/** The consent step: entries still needing a tick, and those the user already allowed. Both are empty
 *  for a built-in, and for a recipe that touches only its inputs and its own results. */
export interface Consent {
  needed: Grant[];
  granted: Grant[];
}

export interface RecipeInfo {
  id: string;
  name: string;
  version: string | null;
  description: string | null;
  /** A glyph name for the card and the breadcrumb. */
  icon?: string | null;
  source: RecipeSource;
  /** Absent from an older engine: read as "shelf". */
  shelf?: ShelfState;
  author?: RecipeAuthor | null;
  /** Absent from an older engine: read as none. */
  tags?: string[];
  /** Set on a duplicate of a catalog recipe; absent or null otherwise. */
  from?: RecipeOrigin | null;
  /** The consent step. Absent from an older engine: read as nothing needed, nothing granted. */
  consent?: Consent | null;
  /** True when the recipe ships a saved example result to look at (no model needed). */
  example?: boolean;
  path: string;
  inputs: Record<string, InputSpec>;
  modelSlots: Record<string, ModelSlot>;
  permissions?: Permissions | null;
  steps: StepInfo[];
  outputs: Record<string, OutputSpec>;
  /** The strip as drawn: declared stages with anything unassigned placed, or one implied "Make". */
  stages: Stage[];
}

/** One chapter of a recipe, as the user reads it in the strip. */
export interface Stage {
  id: string;
  name: string;
  why?: string | null;
  inputs?: string[];
  steps?: string[];
  outputs?: string[];
  /** A run keep: what this stage holds on to between runs. */
  keep?: { refresh?: "always" | "when_sources_change" | "manual" } | null;
}

/** What each run changed in a stage's keep. */
export interface JournalEntry { runId: string; at: string; added: number; replaced: number; removed: number; reason: string }

export interface KeepRecord { id: string; data: unknown; runId: string; writtenAt: string }

/** A kept stage's state for the inputs at hand. */
export interface KeepState {
  stage: string;
  records: number;
  builtAt: string | null;
  /** Whether a run started now would skip the stage; null when it is not built yet. */
  fresh: boolean | null;
  refresh: "always" | "when_sources_change" | "manual";
  last: JournalEntry | null;
}

/** A recipe picked for import, held in quarantine until the user says Add or Discard. */
export interface ImportPreview {
  token: string;
  info: RecipeInfo;
  findings: Finding[];
  sizeBytes: number;
}

export interface RecipesResponse {
  recipes: RecipeInfo[];
  errors: { path: string; error: string }[];
  /** Ids with runs in the history but no recipe folder any more; a new recipe never takes one. */
  reserved?: string[];
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
  /** One line per stage that has run before: what it took when it did. */
  stages: StageEstimate[];
}

/** What one stage took, averaged over the runs in which it really ran; `once` means it is kept
    and will not run again until its sources change or it is rebuilt. */
export interface StageEstimate {
  id: string;
  basedOnRuns: number;
  avgDurationMs: number;
  avgTokens: number;
  once: boolean;
}

/** A tool the runtime has, as shown to a recipe author. */
export interface ToolInfo {
  id: string;
  description: string;
  /** JSON Schema of the inputs object, when the tool describes itself. */
  parameters: { properties?: Record<string, { type?: string; description?: string }>; required?: string[] } | null;
  /** What the output looks like, in words. */
  returns: string;
  cacheable: boolean;
}

/** A recipe as it is on disk, for editing: recipe.yaml as JSON plus its prompt and schema files. */
export interface Draft {
  yaml: RecipeDoc;
  files: Record<string, string>;
}

/** What the checker says about a draft. A problem blocks saving; a note is worth a look and may carry a
 *  fix the engine can apply (`fix` names it, `fixLabel` is the button's wording). */
export interface Finding {
  text: string;
  severity: FindingSeverity;
  fix?: string | null;
  fixLabel?: string | null;
}

export type FindingSeverity = "problem" | "note";

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
  stages?: Stage[];
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
    /** Written by the engine after a model answers: what each managed, by model id. A patch that leaves it out keeps it. */
    speed?: Record<string, ModelSpeed>;
  };
  /** The Codex CLI as a cloud provider: off unless turned on here. */
  codex?: { enabled?: boolean; path?: string; models?: string[] };
  /** Any OpenAI-compatible service by API key (the key lives in the keychain). Off unless turned on. */
  openai?: { enabled?: boolean; baseUrl?: string; models?: string[] };
  /** Model pins per recipe: recipe id → slot → model id. */
  pins?: Record<string, Record<string, string>>;
  /** The planner: a pinned model, or none for Auto (local only); and what Try planning measured per model id. */
  planner?: { model?: string; measured?: Record<string, PlanMeasurement> };
  firstRun?: boolean;
  /** Is there a newer Stillroom: the daily check on launch (off unless turned on) and what the last check found. */
  updates?: { daily?: boolean; last?: UpdateCheck };
}

/** What asking the release feed found. */
export interface UpdateCheck {
  current: string;
  latest: string | null;
  newer: boolean;
  url: string | null;
  notes: string | null;
  publishedAt: string | null;
  checkedAt: string;
  /** Why there is no answer: offline, GitHub refused, the feed changed shape. */
  error: string | null;
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

/** One of the engine's own prompts (the planner), with the user's revisions. */
export interface PromptVersion { n: number; savedAt: string | null }
export interface PromptState {
  name: string;
  title: string;
  purpose: string;
  /** The text in use right now. */
  text: string;
  /** The revision in use; 0 is the original. */
  current: number;
  original: string;
  versions: PromptVersion[];
}

/** What Try planning recorded for one model: how long the strip call took, whether its answer checked clean, when. */
export interface PlanMeasurement { ms: number; clean: boolean; at?: string }

/** One model that could plan, with the cost said up front. */
export interface PlannerCandidate {
  modelId: string;
  /** The bare name, for the picker's label. */
  name: string;
  parameterSize: string | null;
  /** The weights on disk; null when the provider does not say (cloud models). */
  sizeBytes: number | null;
  /** Whether the weights fit comfortably in this Mac's memory (at most about 60 % of RAM). */
  fits: boolean;
  /** Auto's order, 1 first. */
  rank: number;
  /** What Try planning measured, if it ran. */
  measuredMs: number | null;
  measuredClean: boolean | null;
  /** Why it stands where it does, in words. */
  reason: string;
  /** True when the model runs on this Mac. A cloud model is listed only because you turned it on. */
  local: boolean;
}

export interface PlannerPreview {
  resolution: { slot: string; modelId: string; via: "pinned" | "default" | "recommended" | "capability"; reason: string } | null;
  problem: string | null;
  /** Physical RAM of this Mac; null when it could not be read (every model then counts as fitting). */
  memoryBytes: number | null;
  /** Every model that could plan, in Auto's order; cloud ones last, flagged local: false. */
  candidates: PlannerCandidate[];
}

/** Try planning's answer: the planner's first call on a fixed sample goal, timed and checked. */
export interface TryPlanningResult { ms: number; clean: boolean; problems: string[] }

/** The planner's three kinds of model call: the stage list (strip), the custom stages (steps), a fix round. */
export type PlanPhase = "strip" | "steps" | "fix";

/** On every planner event: how long the plan has run and the tokens the model has written so far.
 *  Optional because an older engine sends neither. */
export interface PlanClock {
  elapsedMs?: number;
  tokens?: number;
}

export type PlanEvent =
  | ({ kind: "choosing" } & PlanClock)
  | ({ kind: "thinking"; attempt: number; of: number } & PlanClock)
  | ({ kind: "checking"; attempt: number } & PlanClock)
  | ({ kind: "fixing"; attempt: number; problems: (string | Finding)[] } & PlanClock)
  /** A model call starts; the window clears and the header names it. */
  | ({ kind: "phase"; which: PlanPhase; attempt: number; of: number } & PlanClock)
  /** Where the model call is before and while text arrives. */
  | ({ kind: "model"; phase: CallPhase } & PlanClock)
  /** More of what the model is writing, a few times a second at most. */
  | ({ kind: "delta"; text: string } & PlanClock)
  /** The planner is finished, saved or not. */
  | ({ kind: "done" } & PlanClock);

export interface PlanOutcome {
  draft: Draft;
  /** What the checker still finds; no problems among them means the draft can be saved.
   *  Older engines send plain strings; `toFindings` reads those as problems. */
  problems: (string | Finding)[];
  attempts: number;
  modelId: string;
  promptTokens: number;
  completionTokens: number;
  durationMs: number;
  /** Everything the model wrote, every call, with a "── call N (…) ──" line before each. Older engines omit it. */
  transcript?: string;
}
