# 0.3.1: first use, the planner at work, and recipe ids

Status: proposal, revision 1, 2026-11-02, from Jeremy's first from-scratch install of 0.3.0 through the DMG. Read `BROWSING-PLAN.md` for the shelf and the catalog, `STAGES-PLAN.md` for stages, and `PLAN.md` §5 and §6 for model slots and the Prompt Stack. Six items; the first two were authorised on the spot and ship first, the other four are the plan.

## 1. An empty shelf leads with New recipe (authorised, shipped with this plan)

Today the empty shelf's primary (green, glowing) button is **Browse the catalog** and **New recipe** is plain. Swap them: a person with nothing on the shelf is most likely about to make something; the catalog is the quieter alternative. Nothing else changes.

## 2. Recipe ids: deleted recipes free their id, and the history follows (authorised, shipped with this plan)

A new recipe's id is its name slugified with a suffix until free (`report`, `report-2`). Archived recipes are still folders on disk, so their ids stay taken. Deleted ones are not: the folder goes, but the run history under that id stays, so the next "Report" gets `report-2` back and inherits a dead recipe's runs, estimates, and "last run" tag.

- **Minting treats an id with runs as taken.** `list_recipes` returns `reserved`: ids that have runs but no folder. The frontend's taken set is folders plus reserved. Ids stay readable; `report-2` is skipped if it ever ran.
- **Delete marks the runs.** `delete_recipe` rewrites the recipe's runs so the history reads "Report (deleted)". The runs stay, since they hold results the person may still want.
- **A new save never overwrites.** `save_recipe_draft` takes `expectNew`; the create paths (blank, goal, copy) send it, and a folder that already exists is refused with "A recipe with the id “x” already exists; pick another." Edits in place send nothing and behave as today.

## 3. New recipe: the three ways to start become tabs

**Status 2026-11-02: shipped** (kit `dec30ed`, app `b06eab9`; `docs/screenshots/first-use-new-recipe-tabs-cost-line.png`, `first-use-blank-tab.png`). The Draft it button now also says what it still needs (a name, a goal, a recipe to copy) instead of sitting dim.

The three choices (A goal · Blank · A copy of a recipe) are boxes in a row inside the zone. They should read as **tabs that sit on the zone's top edge**: the selected tab joined to the panel below it (no bottom edge, the panel's ground colour, a faint glow), the other two recessed and dimmer, all three touching the zone's border at their outer ends so the row looks cut into the frame rather than placed on it. The one-line hint for each start moves under the tab row as the panel's first line, so the panel always says what the selected start does.

Where it lives: the look comes from the kit, so this is a kit change first: `Zone` gains a `tabs` slot rendered flush with its top border, and a `Tabs` component (`role="tablist"`, arrow keys, `aria-selected`) that the Dial does not already cover (the Dial is a ring of places; this is a tabbed panel). Stillroom then uses `Zone tabs={…}` in `NewRecipe.tsx`; the CSS for `.choices` goes away. Build the kit (`pnpm build` in `../overtone-foundry-app-kit`) before Stillroom. One session. Verified by a screenshot of each tab selected, in the three themes.

## 4. Which model plans: a suggested planner, and the cost said up front

**Status 2026-11-02: shipped.** Live on this Mac: Auto picked qwen3.8:27b-mlx (18.2 GB, fits); "measure it" recorded 18 s clean; Try planning in Settings recorded GLM 4.7 Flash at 1 m 16 s clean, so the rank's pick stands (`docs/screenshots/first-use-settings-try-planning.png`). Not done: the Settings › Pull a model zone still suggests the generic starter; the welcome has the planner-sized one.

Today "Plan with · Auto" resolves the planner slot like any slot with `preference: accuracy`, which means **the largest installed model that answers in a fixed shape**. On this Mac that is a 49 GB coder model: slow, and not the best drafter. Jeremy reaches for GLM 4.7 Flash (q8, 30 B); the UI gave no reason to.

- **A planner rank, not "largest".** The planner slot gets its own tie-break: among installed models with `text` and `structured_output`, prefer ones that **fit comfortably in memory** (weights at most about 60 % of physical RAM; above that, generation falls off a cliff), then a short family rank for drafting in JSON (qwen3 14 B and up, glm-4.x flash, gpt-oss, gemma3 12 B and up, then the rest by size), then size. Auto's line in New recipe says why: "Auto · glm-4.7-flash (fits in memory, answers in a fixed shape)".
- **Measured, not guessed.** Settings › Models gains **Try planning** beside Try: it runs the strip call on a fixed sample goal and records seconds and whether the answer checked clean. Auto prefers a model with a clean measured plan under two minutes over any unmeasured one. New recipe shows "the last plan with this model took 2 m 10 s" once there is a number.
- **Say the cost before Draft it.** Under Plan with: the model's size, whether it fits in memory, and the last measured time; amber when it does not fit ("29.9 GB on a 36 GB Mac: this will be slow; a smaller model plans in a fraction of the time").
- **A starter suggestion.** The welcome's model step and the pull suggestion name one planner-capable model in the 8 to 14 B range for machines with 16 to 32 GB, so a first run never lands on a model that cannot plan.

Rust: `planner_slot()` preference and a `rank_for_planning` in `resolve.rs`; `store` setting `planner.measured.<model>`; a `try_planning` command. Frontend: `NewRecipe.tsx` Plan with block, Settings › Models row. Two sessions. Verified live with the four-goal test on the suggested model and a screenshot of the cost line.

## 5. The planner at work: show what the model is writing

**Status 2026-11-02: shipped.** Live: the window showed the model's thinking under "Choosing the stages · call 1 of 1 · 10 s · 273 tokens", Stop mid-stream left "stopped" and re-enabled Draft it, "Writing the custom stages · call 1 of 3 · 54 s · 2663 tokens" followed, and the finished card's note offers "show what it wrote" (`docs/screenshots/first-use-stream-*.png`, `first-use-transcript.png`). Thinking deltas are included, marked (thinking) and (answer).

While the planner runs, the button says "Thinking about the steps…" and nothing moves for minutes. The model is writing JSON the whole time; show it.

- **Streaming in the model layer.** `ModelClient` gains `chat_stream(model, req, on_delta)` with a default that calls `chat` and emits once; the Ollama adapter implements it with `stream: true` (NDJSON lines; `thinking` deltas when `think` is on; `format` still applies). Codex and the OpenAI adapter keep the one-shot default for now.
- **Events.** `PlanEvent` gains `Phase { which: strip | steps | fix, attempt, of }` and `Delta { text }`, throttled to a few per second, plus `elapsedMs` and the token count so far on every event.
- **The window.** Under Draft it, a dim, read-only, auto-scrolling block in the mono face: the last dozen lines of what the model has written, a blinking caret while it streams, a header line "Choosing the stages · call 1 of 2 · 48 s · 612 tokens", then "Writing the custom stages · call 2 of 2". The JSON is shown as it is (it is what is happening; the text inside it is readable: stage names and whys appear first). When a fix round starts, the block clears and the header says "Fixing 2 problems · try 2 of 3". Stop still works mid-stream. On the first chunk, the "Thinking about the steps…" label becomes the header.
- **Nothing hidden.** The whole stream is kept for the run and shown on the card's note ("Drafted by … in 2 m 10 s · show what it wrote").

Rust: `crates/models` (provider trait, ollama), `crates/core/planner.rs` (events), `src-tauri` (emit). Frontend: `NewRecipe.tsx`. Two sessions. Verified live: a plan with GLM 4.7 Flash showing the stream, Stop mid-stream, and a screenshot.

## 6. The goal screen says what is baked in and what is asked for each run

**Status 2026-11-02: shipped** (app `b06eab9`).

The two fields do not tell a first-time user whether to paste their document or make the recipe ask for one. New wording:

- **What should this recipe do?** · sub: "say what comes in each time and what should come out". Placeholder: "Ask me for a folder of PDFs and a question each time, and answer from the documents with the page it came from." (shorter than today's; the example names an input and an output).
- **Built into the recipe** (was "Anything it should know") · sub: "optional · pasted here, it is baked into the recipe's prompts and used every time it cooks". Placeholder: "House-style notes, a fixed list of links, a template to follow. Leave this empty when each run brings its own file or links; say so in the goal instead." One quiet line under the field: "Material pasted here is part of the recipe and travels with it when shared; what varies per run should be asked for, not pasted."
- Set up already offers `make this part of the recipe` and `ask each run instead` for any input (STAGES-PLAN §2.4); the goal screen's line names that so a wrong first guess is cheap.

Frontend only, `NewRecipe.tsx`; the planner prompt's wording for material stays. Half a session. Verified by Jeremy's read-through.

## 7. Order and release

**2026-11-02:** all six items shipped on `first-use` and merged to `main` as 0.3.1. The goal that failed three times under 0.3.0 (a learning-kit builder that saves into an Obsidian vault) drafted clean on the second try in 4 m 40 s: five stages, thirteen steps, a kept stage, four ready-made stages and a custom one (`docs/screenshots/first-use-drafted-learning-kit.png`). The two planner slips it kept making (bare `${step}` references, a step in no stage) are now repaired by the registry before the checker sees them.

| Step | What | Needs the screen |
|---|---|---|
| 1, 2 | shipped with this plan | no (tests only) |
| 6 | wording | no |
| 3 | tabs (kit, then app) | screenshots |
| 5 | streaming and the window | a live plan |
| 4 | planner rank, Try planning, the cost line | a live plan on two models |

Then 0.3.1 through the same release steps as 0.3.0. Items 4 and 5 are the ones that change how the first minute with the planner feels; 3 and 6 are an afternoon between them.

## 8. Decisions to confirm

- The planner's Auto stops meaning "largest" and means "fits, ranks well for drafting, measured fast". Proposed.
- The stream is shown raw (JSON as the model writes it), dimmed, never editable. Proposed; the alternative, a parsed "stage 1: …, stage 2: …" view, can come later on top of the same events.
- "Anything it should know" is renamed "Built into the recipe". Proposed.
