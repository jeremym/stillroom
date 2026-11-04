# Feedback, fit, and talking to the planner

Status: built, revision 2, 2026-11-04 (all five phases on the `feedback` branch; see §10), from Jeremy's test pass of 0.3.3 with a recipe of his own (Jeremy Learn Stuff, planner journal `plans/2026-11-04-115141-jeremy-learn-stuff.md`). Read `FIRST-RUN-PLAN.md` for the goal screen, `STAGES-PLAN.md` for stages and the stage library, `UI-PLAN.md` for the look. This plan covers five things Jeremy hit in one sitting: the dead time while a local model warms up, no idea whether this Mac can run the model it picked, a Set up form whose sections and labels do not read, permissions and required inputs that are unclear until Run refuses, and a planner that made every source required when the goal said "URLs or documents".

## 0. Vocabulary

| Word | Means |
|---|---|
| **Warm-up** | What Ollama does before the first word: loading the model's weights into memory (seconds to a minute, longest after a while idle) and then reading the prompt (prefill). Nothing is written during either. |
| **Fit** | Whether a model's file size sits comfortably inside this Mac's memory. Today the ceiling is 60 % of unified memory (`MEMORY_CEILING`); a model past it swaps and crawls. |
| **Source stage** | A ready-made stage whose only job is to bring material in: `read_links`, `read_folder`, `index_folder`, `watch_pages`. |
| **Fed input** | An input a ready-made stage asks the person for (paste the text to learn) that, in a particular recipe, should come from an earlier stage instead (the brief). |
| **Edit by asking** | Telling the planner what is wrong with a recipe in words and getting a revised draft back, instead of wiring steps on the card. |

## 1. What Jeremy saw, and the cause of each

| # | What he saw | Cause in the code |
|---|---|---|
| 1 | Long waits with nothing moving: before the planner's first word, and between steps of a run. | Run model steps do not stream at all (`Executor::chat_call` uses `chat`, not `chat_stream`), so a step is "running" until it ends. The adapter never asks Ollama whether the model is loaded (`/api/ps`), never sends a warm-up, and never reads `load_duration` or `prompt_eval_duration` from the answer. The planner window shows a clock and "0 tokens", nothing that says *loading* or *reading*. |
| 1b | No warning that a model is too big or this Mac too slow; no estimate until a recipe has run. | Hardware facts exist (chip, memory, GPU cores, ceiling) and measured tokens per second per model are stored, but estimates come only from previous runs (`Store::estimate_for_recipe`). Nothing combines model size, speed, and prompt length into a first guess. Quantization and whether the model is currently loaded are not read. |
| 2 | **NOT BUILT YET** under "Read Documents"; it reads as "this feature does not exist". | It is the keep status word for a kept stage that has not run yet (`keepWord` in `src/stages.ts`). The stage is fully built; the words are wrong. No MCP or extra tool is missing: `fs.list` and `doc.text` are the tools that stage runs. |
| 3 | A numbered section with two fields looks like a loose pile; the field labels are tinier than their own descriptions. | One `h3` holds number, name, estimate tag, and the whole `why` sentence inline; fields follow with no wrapper. The label is display caps at 13 px (`--size-label`); the description after the "·" inherits the 17 px body size. The kit has no group or fieldset component, only `Zone` (a whole section with an h2). |
| 4 | Unsure whether the run may write to "Where to save"; "Needs your documents, what to learn" does not say which section. | Permissions are one prose line; writability is never checked before Run. `missing` is computed over all inputs, not by stage, and the Run button lists labels without the stage number. |
| 5 | The planner demanded a folder *and* pasted text *and* links, though the goal said "URLs or documents". | Two structural gaps in the stage library, not prompt wording: (a) `read_links` and `read_folder` each declare their input as required, and the planner cannot change what a ready-made stage asks for, so "read both" means "require both"; (b) `study_cards` asks for a paste (`material`) and nothing lets the strip say "feed it from the brief", so the recipe asked for notes the brief was meant to be. The engine itself already runs an empty optional loop zero times (the either-or fixture in `draft.rs`). |

## 2. The waiting (1)

**Proposal.** Every model call reports a phase, and the UI shows a progress line for it.

Phases, from the adapter (`crates/models/src/ollama.rs`):

| Phase | When | What the person reads |
|---|---|---|
| `loading` | `/api/ps` did not list the model before the call, until the first chunk arrives | "Loading qwen3.8:27b into memory · 0:14 · first use after a while takes longest" |
| `reading` | model listed as loaded, no chunk yet | "Reading the prompt · 2,300 words · 0:06" |
| `writing` | chunks flowing | "Writing · 140 words · 18 words/s" |
| `done` | final chunk | tokens, and the three durations Ollama returns (`load_duration`, `prompt_eval_duration`, `eval_duration`), stored per model for §3 |

How:

- `ModelClient::chat_stream` gains a phase callback beside `on_delta` (or `on_delta` takes an event enum: `Phase(..) | Text(..)`). The Ollama adapter calls `/api/ps` once before the request (a few milliseconds, local) to decide `loading` versus `reading`. No warm-up call: it would double the wait on a cold model. Remote and non-Ollama providers report `reading` then `writing`.
- `Executor::chat_call` switches from `chat` to `chat_stream` so a run step gets phases and a live word count. The `StepRecord` gains `phase`, `words_so_far`, and `words_per_second`; the observer already fires on every record change, so `run:step` carries it with no new event.
- The planner's `Progress` already has `Phase` and `Delta`; it adds the adapter phase to `Thinking` so the window's header says "Loading the model" / "Reading the goal" / "Writing" before and during the stream.
- UI: the kit `Progress` component takes the line. Determinate when §3 has an estimate for the phase (loading: size ÷ measured load rate; reading: words ÷ measured prefill rate; writing: expected words ÷ measured speed), an indeterminate pulse when not. The Teletype prints the phase line under the step's `▶` line and rewrites it in place; `StepDetails` shows the same.
- Stop stays as it is (cancel drops the stream).

**Not in scope.** Changing Ollama's `keep_alive`; a setting for it can come later if cold loads dominate.

## 3. Fit and estimates (1b)

**Proposal.** A *fit verdict* per installed model, an *estimate* for any step before it has ever run, and an amber notice when the picked model does not fit or this Mac has no GPU.

Facts to add to `InstalledModel` and the hardware record:

- From `/api/show`: `quantization_level`, `family`. From `/api/ps`: `loaded`, `size_vram` (how much sits on the GPU).
- From every real call: `load_duration`, `prompt_eval_count` ÷ `prompt_eval_duration` (prefill rate), `eval_count` ÷ `eval_duration` (speed), kept per model as rolling medians in the `models` settings record next to the existing `speed.<id>`.
- Hardware: `gpu_cores == 0` or no Metal means "no GPU" (an Intel Mac, or a VM).

Fit verdict (`crates/core/src/resolve.rs`, beside `fits_in_memory`):

| Verdict | Rule | Where it shows |
|---|---|---|
| fits with room | size ≤ 40 % of memory | Settings › Models row, Set up next to the pick |
| fits, tight | 40–60 % | same, dim |
| too large for this Mac | > 60 % | amber notice on Set up and the goal screen: "qwen3.8:27b is 17 GB; this Mac holds about 14 GB comfortably. It will run but slowly. Pick a smaller model, or let Auto choose." |
| no GPU | hardware says so | amber notice once per session: "This Mac has no GPU Ollama can use; every model will be slow. Expect minutes, not seconds." |

Estimate (`crates/core/src/estimate.rs`, new; `Store::estimate_for_recipe` falls back to it):

```
step time ≈ load (0 if loaded; else size_bytes ÷ load rate, default 1 GB/s)
          + prompt words ÷ prefill rate (default 300 words/s on Apple silicon, 40 without a GPU)
          + expected output words ÷ speed (measured, else 15 words/s on Apple silicon, 2 without a GPU)
```

Expected output words come from this step's previous runs; else from the step's shape (a schema answer ≈ 300, a brief ≈ 900, a short answer ≈ 150; a loop multiplies by its item count once inputs are known). Prompt words are countable at Set up time from the inputs given. The stage estimate is the sum; Set up says "about 6 min, a guess from this Mac and the model" until the first run replaces it with "from the last 3 runs". The same numbers feed §2's bars.

Auto's rank already prefers "fits, ranks well, measured fast" for the planner; the verdict makes the same rule visible for every slot.

## 4. Words, groups, and labels (2, 3)

**Wording.** `not built yet` becomes `nothing kept yet · builds on the first run` (dim); the Recipe page's version, `keeps nothing yet`. Grep the status words for anything else that sounds like an unfinished feature.

**Stage groups on Set up.** Each stage with inputs becomes a bordered group:

```
 ┌─ 2  YOUR DOCUMENTS ────────────────────────────── about 2 min ─┐
 │  Read every file in the folder you point to … so nothing      │
 │  is missed.                                                     │
 │                                                                 │
 │  YOUR DOCUMENTS *                                               │
 │  [ Choose… ]  ~/Reading/graph-theory                            │
 │  A folder of .pdf, .docx, .md, or .txt files. Subfolders too.   │
 │                                                                 │
 │  WHAT DO YOU NEED TO KNOW?                                      │
 │  [                                                            ] │
 │  A question or a sentence about what matters.                   │
 └─────────────────────────────────────────────────────────────────┘
```

- Head: the number and name on one line, the estimate tag at the right; the `why` as its own line under the head at `--size-small`, dim. Not inline in the h3.
- Rim: amber (`amber-rim`, the halo the kit already has) while a required input in the group is empty; plain bezel once filled; plain bezel with "optional" in the head when nothing in it is required. The rim is the answer to "which section is required".
- Field: label in display caps at `--size-label` as now, with `*` for required; the control; the description **under** the control at `--size-small`, dim. Never on the label line.
- Kit: a `Group` component (`section.group` with `.group-head`, `.group-why`; variants `amber-rim`, `optional`) and a `.field .desc` rule, both in `@overtone-foundry/app-kit`, since Settings and the consent zone want the same shape. Stillroom's `.consent-zone` becomes a `Group amber-rim`.
- The Run button names the stage: "Run · 2 Your documents needs a folder". The Strip's "needs you" cell links to that group as it does now.

## 5. Permissions you can see (4)

**Proposal.** The prose line becomes rows, checked before Run:

| Row | Check at Set up | If it fails |
|---|---|---|
| Reads: the files and folders you chose · 2 folders | each exists and is readable | amber "cannot read ~/x" |
| Writes: Where to save · ~/Study/graph-theory | folder exists (offer to create it) and a probe file can be written and removed | amber "cannot write here; choose another folder" and Run stays disabled |
| Reaches: the sites in your links · 3 hosts | hosts listed | — |
| Nothing else is touched. | | |

The check runs when the input changes, in Rust (`permissions.rs`), so the answer is the same one the executor would give. Full Disk Access and the macOS folder prompts stay where they are (Settings › Permissions).

## 6. The planner (5)

### 6.1 Either-or sources

A stage definition may declare `source: true`. When a strip holds two or more source stages, `expand` marks each one's source input `required: false` and the draft gains:

```yaml
inputGroups:
  - anyOf: [urls, folder]
    label: Give links, a folder, or both.
```

`Draft::check` requires every name in a group to be an optional input. Set up shows the group's label across the two stage groups and counts the requirement as met when any one is filled; the Run button says "needs links or a folder". The executor already skips what was not given ("nothing given for urls").

### 6.2 Fed inputs

A stage definition may say which of its inputs can be fed from an earlier stage:

```yaml
inputs:
  material: { type: text, label: What to learn, …, fed_by: ["${steps.brief_write.output}", "${steps.file_text.output.text}"] }
```

When a listed step is present in the recipe, `expand` drops the input and rewrites the stage's steps to read from that step (the `name?:` mechanism in `resolve_optional_inputs` already does half of this). `study_cards` after `write_brief` then reads the brief; after `summarize_file`, the file's text; alone, it still asks for a paste. The strip prompt's library listing gains "fed by `write_brief`, `summarize_file`" so the model knows the order matters.

### 6.3 Planning in smaller pieces

The planner is two calls today (strip, then all custom stages in one answer). For a model the fit verdict calls small or tight, the second call becomes **one call per custom stage**: each answer holds one stage's steps, prompts, and schemas, with `already_built` listing what the earlier stages return. Smaller answers, smaller context, and a checker note points at one stage. The merge reuses `merge_partial`. Large models keep the single second call. Decision to confirm whether to make per-stage the only mode.

### 6.4 Edit by asking

On the Recipe card, beside the editor, **Ask for a change**: a text box ("The quiz should come from the brief, not from pasted notes. Drop the voice script.") and a Plan button.

- One planner call with a new prompt `prompts/edit.md`: the current recipe (yaml and files), the instruction, the stage library, and the rule "return the whole recipe with only what the instruction needs changed". Then `Draft::check` and the fix loop as for a new draft. Same slot, same streaming window, same journal under `plans/`.
- The result is shown as a **change list in words** before anything is saved: stages added or removed, inputs added or removed, steps renamed, prompts rewritten (with the old and new prompt side by side).
- Save as copy is the default (the original stays on the shelf); Save in place is offered for the person's own recipes. A built-in is never edited in place; Ask for a change on one makes a copy first, which is Jeremy's "clone it, then talk to it".
- The card editor stays for the person who wants to wire by hand.

## 7. Phases

| Phase | Builds | Done when |
|---|---|---|
| A. Words and rims | §4 and §5: the keep wording, `Group` and `.field .desc` in the kit, stage groups with the amber rim, descriptions under controls, the Run button by stage, permission rows with the write probe | Jeremy's recipe reads at a glance; a folder he cannot write to is caught before Run |
| B. The waiting | §2: adapter phases with `/api/ps`, the three durations stored, run steps streamed, the progress line on Running and on the planner window | Nothing sits still for more than a second on screen while a model works |
| C. Fit and estimates | §3: quantization, loaded flag, fit verdict, the estimate model, the two amber notices, Set up estimates with no runs | A 27b model on a 24 GB Mac is warned about; a first run has a guess within 2× |
| D. Planner structure | §6.1–6.3: source stages and `inputGroups`, fed inputs, per-stage planning for small models; `study_cards` and the source stages updated; a compat fixture for Jeremy's goal | Jeremy's goal drafts with links *or* a folder and cards from the brief, on the first try |
| E. Edit by asking | §6.4 | "Drop the voice script" on a copy of Jeremy's recipe produces a clean draft and a readable change list |

A then B first: both are what Jeremy sees on every run. D fixes the recipe he is holding; C makes the waiting honest; E is the new way of working.

## 8. Decisions (confirmed 2026-11-04)

1. **No warm-up call.** Report `loading` from `/api/ps` and the first chunk's arrival; never pre-load. One cold wait, not two. (Jeremy: yes.)
2. **Fit is about the weights, not the prompt.** The verdict compares the model file to memory: room up to 40 %, tight to 60 %, too large past it (the ceiling Auto already uses). What the person types changes the *time*, not the fit: a vague goal makes a long answer, and that is what the estimate and the progress line are for, with a context-length note when the prompt plus the expected answer would not fit the slot's context window. (Jeremy left this to the plan.)
3. **Required shows as an amber rim** on the stage group while something in it is empty, plus `*` on the label; optional stages say "optional" in the head. No other marker. (Jeremy: yes.)
4. **Descriptions go under the control** at the small size, never inline after the label; a kit rule, so Settings follows. (Jeremy: yes.)
5. **Either-or is `inputGroups.anyOf`**, written by `expand` when two source stages are picked; the planner never writes it by hand. (Jeremy: yes.)
6. **Per-stage planning for small and tight models** (the fit verdict decides), the single second call for models with room; the plan journal records which mode ran, and if live runs show per-stage drafts cleaner for every model, it becomes the only mode. This is only the planner (drafting a recipe), not a run. (Jeremy left this to the plan.)
7. **Edit by asking** returns a whole recipe, shows a change list in words, never saves without a click, and never edits a built-in in place: it makes a copy first. (Jeremy: exactly as envisioned.)

## 9. Risks

- Streaming run steps changes the fingerprint of nothing, but the executor's timeout and cancel paths need the same tests they have for `chat`.
- `/api/ps` is Ollama-only; the OpenAI-compatible adapter reports `reading` only, so its bar is a pulse.
- The estimate defaults are guesses until measured; the words "a guess" stay on screen until a run replaces them.
- `inputGroups` is a format change: a migration entry, the schema, `runtime.minVersion` 0.4.0 for recipes that carry it.
- Edit by asking is one more place a small model can produce a broken draft; the checker and the change list are the guard, and the original is never touched.

## 10. Status (2026-11-04)

All five phases are built on the `feedback` branch, each its own commit, every check green (Rust 169 + 6 compat + 13 adapter tests, 165 frontend tests, `tsc`, the Vite build).

| Phase | Commit | What landed | Verified |
|---|---|---|---|
| A. Words and rims | `10854dc` | Kit `Group` (amber rim, why line, aside) and `span.desc`/`span.req`; Set up as stage groups; descriptions under controls; Run names the stage, or "needs you in stages 1, 2, 4"; permission rows with a write probe (`core::probe`); "nothing kept yet · builds on the first run"; a kept stage that still needs you says so first | On screen, `docs/screenshots/feedback-setup-*.png` |
| B. The waiting | `635de70` | `CallPhase` and `StreamEvent` in the model trait; Ollama asks `/api/ps` and reports loading / reading / writing with its load and read times; run steps stream; the record carries phase, prompt words, words so far, pace; the teletype's phase line, the step details' busy bar (kit `Progress busy`), the planner header's phase | Live: loading at 0 ms, writing at 2.05 s against Ollama's 1.71 s load + 0.27 s read (`crates/models/tests/phases_live.rs`) |
| C. Fit and estimates | `e7a2c61` | quantization and loaded on a model, GPU on the hardware record; `Fit` (room / tight / too large) in reasons, candidates, Settings; prefill rate and load time recorded per model; `core::estimate` guesses a first run by stage; the two amber notices on Set up and the goal screen | Unit tests; this Mac (M3 Ultra) reads as GPU, 4 models with quantization |
| D. Planner structure | `a13b977` | `inputGroups` in the format (schema, checker, engine, Set up, strip, 0.4.0 floor); `source` and `feeds` in the stage library; the strip prompt says both; one call per custom stage for a small or tight model; Jeremy's goal as a compat fixture | Fixture: links or folder either-or, cards fed from the brief; live replay of the goal in §10.1 |
| E. Edit by asking | `867ac5f` | `edit` prompt, `Planner::edit`, `changes()`, the `edit_recipe` command and journal entry, the Ask for a change zone on the Recipe card with the change list, prompts before and after, Save as a copy, Save in place (own recipes only) | Unit and UI tests; a live edit waits for the screen |

### 10.1 Live, the same afternoon

- **Jeremy's goal through the planner** (headless harness, qwen3.8:27b-mlx, one strip call and one composing call): clean on the first try in 5 min 48 s. Links and the folder came out optional with one either-or group, the study cards read the brief and ask for no paste, the floor is 0.4.0. The composing call had re-declared a dead "Material for study cards" input because it was told the library stage's original inputs; it is now told what the expanded stage still asks for, and the merge drops an input nothing reads. The recorded answers are the compat fixture `two-call-links-or-folder-brief-cards.json`.
- **Set up of that draft** (`docs/screenshots/feedback-setup-either-or.png`, `feedback-setup-guess.png`): both source stages amber with "Give at least one of these: Pages to read, Your documents. Either is fine; both is fine too.", the labels say "(optional)", the estimate says "about 6 min, a guess from this Mac and the models picked above" with every stage's share.
- **Running** Describe a document on the README (`feedback-running-loading.png`, `feedback-running-writing.png`): "… loading the model into memory · the first call after a while takes longest" at 5 s, "… writing · 479 words · 43.4 words/s" at 17 s, done at 23 s.
- **Settings › Models** (`feedback-settings-fit.png`): Fit says "fits with room" for three models and "fits, but tightly" for the 80 GB one on this 96 GB Mac; the quantization sits beside the size; the loaded model wears "in memory".
- **Ask for a change** on the live draft (`feedback-ask-for-a-change.png`, `-running.png`): "Drop the voice script and its saved file; keep everything else as it is." The window said "reading the goal" at 9 s; the result is in §10.2.

Decision 6 in practice: on this Mac every installed model has room (M3 Ultra), so per-stage planning will not trigger here; it is exercised by the unit test and waits for a smaller Mac or a pinned small model to be judged live.
