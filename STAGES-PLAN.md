# Stages: making a recipe explain itself

Status: proposal, revision 3, 2026-10-31, after Jeremy's second round of answers; phase 1 begins. Read `PLAN.md` for the product and `UI-PLAN.md` for the look; this plan changes how a recipe is **structured, remembered, and presented**, so that a user who typed one sentence sees a clear strip of stages, understands why each exists, and never meets a graph unless they open the Recipe card.

## 0. Vocabulary (the words to correct me with)

| Word | Means | Not to be confused with |
|---|---|---|
| **Recipe** | The whole process, saved as a folder; the planner writes one from a goal. | |
| **Stage** | One chapter of a recipe, shown as a numbered cell in the strip: a name, a **why**, the inputs it asks for, the steps it runs, and optionally a keep. Stages are different for every recipe; the planner decides them. | **Step**: one box on the map, inside a stage (a program, an AI step, a loop, an approval). |
| **Material** | Something baked into the recipe itself because it is always the same: a 200-page handbook, a fixed list of links, house-style notes. Lives in the recipe folder (`assets/`), travels with the recipe. Set up shows it as `built in · change`. | **Input**: something asked for at each run because it varies (today's file, this question). |
| **Run keep** | What a stage holds on to between runs: records (JSON), files, and optionally an index for searching. Built by runs, mutable, every change written to its journal. "Keep" alone in code and tool ids (`keep.put`); "run keep" wherever a person reads it. | The step cache (invisible reuse of an unchanged step's output), and material (which is part of the recipe, not produced by runs). |
| **Journal** | The record of what a keep's runs changed: when, which run, what was added, replaced, or removed, and why (the stage's why plus the step that did it). | The run history (one line per run). |
| **Checkpoint** | A stage that waits for the user (it contains an approval step). Amber in the strip. | |
| **Strip** | The row of stage cells under the Dial. Drawn from the recipe, so it is different for every recipe. | **Dial**: the run's life (Recipe · Set up · Running · Results · Recipe card). |
| **Why** | One plain sentence per stage saying what it does and why it exists. The planner writes it; you can edit it. | |

## 1. In one screen

**The problem.** A recipe today is a flat list of steps that all run every time. The Dial shows a run's *life*, but nothing shows the work's *shape*: what is prepared and remembered, what the user supplies, what is made, where they are asked. Nothing says *why* anything happens. Ask Your Documents exposes it: an index must be built before any question, kept between questions, refreshed when files change. A daily news recipe exposes the same thing from another side: it must remember what it has already seen. A research recipe that reads websites should be able to keep what it parsed, not refetch it every run.

**The proposal.** A recipe declares **stages**: an ordered strip of chapters, each with a why, grouping its inputs and steps. Any stage may have a **keep**: a mutable, recorded store that outlives the run, which the stage's steps read and write through generic store tools, and which later stages may read. Keeps are one generic thing, not a set of special cases: a vector index, a news log, a folder of parsed web pages, and a transcript are all keeps with different contents. The strip is drawn from the recipe (so the planner builds it from the goal), with a status word per stage. The engine runs one stage at a time, decides whether a stage with a keep needs to run again, and journals every change. Document tools (text out of PDFs, chunking, embeddings) plus the store's search view make Ask Your Documents the first recipe that needs all of it.

```
 ① ADD DOCUMENTS   ② BUILD THE INDEX      ③ ASK          ④ ANSWER
   a folder          keep · 42 files         a question     with sources
   needs you         · fresh Oct 31           needs you      —
                     · rebuild
```

## 2. The flow, restated

1. **Goal.** New recipe › A goal: the user types what they want, pastes material, presses Draft it.
2. **Plan.** The planner answers with a recipe *and its stages*: names, whys, which inputs belong to which stage, which steps, and which stages keep something. Different goals produce different stages. The checker reports problems in words; the planner fixes them.
3. **Strip first.** The draft opens on the strip (the wizard), with each stage's why and what it will make; the graph is folded underneath. The user renames, reorders, or removes stages here, or opens the card for the machine.
4. **Set up by stage.** Inputs are grouped under their stage; material shows as `built in · change`, and any input offers `make this part of the recipe` (which copies the file or text into the recipe's `assets/` and turns the input into material), while material offers `ask each run instead`. A stage with a keep shows its state (`not built yet`, `fresh · 42 files · Oct 31`, `stale · 3 files changed`, `212 records`) and a `Build now` or `Rebuild` button, so preparing can happen before the run that uses it.
5. **Run stage by stage.** The teletype prints a stage heading with its why. A stage with a keep runs according to its refresh rule (below); when it does not need to run, the cell says `ready · kept` and its outputs are loaded as if it had run. Inside a stage, the existing step cache and content-matched loops still skip unchanged work. Every write to a keep goes to the journal.
6. **Results by stage.** Outputs grouped by stage. A stage whose inputs can change without touching earlier stages gets **Ask again**: the field inline, a button that runs from that stage with everything before it kept.
7. **Recipe card.** The graph with stage bands, the inspector showing the stage, its why, and (for a keep) its journal.

## 3. Stages in the recipe format

Optional, so every existing recipe stays valid. A recipe without stages gets one implied stage, "Make", so the strip still draws.

```yaml
stages:
  - id: documents
    name: Add your documents
    why: The folder you choose is the only thing the recipe reads.
    inputs: [folder]
  - id: index
    name: Build the index
    why: Each passage is turned into numbers (embeddings) so a question can find the passages that match it.
    keep:
      refresh: when_sources_change     # always | when_sources_change | manual
      search: embeddings               # the keep can be searched by meaning
    steps: [list_files, index_files]
  - id: ask
    name: Ask
    why: Your question is matched against the index; only the best passages go to the model.
    inputs: [question]
    steps: [search, answer]
  - id: answer
    name: Answer
    why: The answer cites the passages it used, so you can check it.
    outputs: [answer, sources]
```

Rules the checker enforces: every step, input, and output belongs to exactly one stage (unassigned ones fall into "Make"); stages are ordered and a step may only read from its own or earlier stages; a stage with a keep is the only writer of that keep (later stages read it); an approval step makes its stage a checkpoint. Steps keep `id`, `kind`, `inputs` exactly as now; the stage lists its steps, so the editor and the planner deal with one list.

## 4. Keeps and the journal

A keep is the generic "data behind a recipe". One mechanism; the contents differ.

- **What it holds.** Records (JSON documents with an id, written and read by the stage's steps), files (anything a step wrote into the keep), and, when `search` is set, an index over records (embeddings for search by meaning; later, other kinds).
- **Where.** `<app data>/recipe-data/<recipe id>/<stage id>/` as SQLite plus files. Never beside the user's files; deleted with the recipe; never inside a shared recipe (a shared recipe arrives with every keep empty).
- **Store tools** (generic, any stage with a keep may use them; the planner learns them like any tool): `keep.put` (write or replace records by id), `keep.get`, `keep.query` (filter by fields, newest first, limit), `keep.search` (by meaning, when the keep is searchable), `keep.list` (what is there), `keep.remove`. Records written with `keep.put` carry the run id and time; the journal is derived from them, not kept separately.
- **Sources and freshness.** A keep records the **source fingerprint** of what built it (a folder: the file list, sizes, times; links: the list; text: its hash). The refresh rule says when the stage runs again: `when_sources_change` (default for indexes and parsed material), `always` (default for logs, gathering, anything that reads the world), `manual` (only Build now or Rebuild). The strip shows the state in words.
- **Mutable and recorded.** A stage may read its own keep and change it (the news log reads what it has seen, appends what is new; a web-reading stage keeps the parsed pages and refetches only new links). The journal shows what each run did to the keep, in words: `run 8f2c · added 14 records · replaced 2 · reason: 3 files changed`. Rebuild empties the keep and journals that too.
- **Permission.** A stage with a keep adds one line to the recipe's permissions, shown in Set up: "Keeps an index of your folder on this Mac, in Stillroom's data folder; deleting the recipe deletes it."

Nested recipes as stages (a stage that runs another recipe, the `recipe` step kind that the schema already parses) fit this model without changes to it: the inner recipe's keeps are its own. Not in this round; noted in §8.

## 5. The strip

Drawn under the Dial on Recipe, Set up, Running, and Results. The Dial says where the run is in its life; the strip says where the work is in its shape.

- Each stage is a numbered cell: glyph, name, one status word, a fold for the why. Cells are a `role="tablist"` like the Dial; clicking one scrolls Set up to that stage's inputs or Results to its outputs.
- **Status words:** `needs you` (an input of this stage is empty), `not built yet`, `fresh · 42 files · Oct 31`, `stale · 3 files changed`, `212 records · since Oct 3` (an append-style keep), `running 3 of 7`, `ready · kept` (skipped because fresh), `done`, `waiting` (a checkpoint with items to approve, amber), `skipped`, and a dim `—` for stages with nothing yet.
- **Set up** groups inputs by stage with the why above the fields; a keep shows its state and `Build now` / `Rebuild` beside Run. RUN still says what it needs.
- **Running:** the current cell glows; the teletype prints `── ② BUILD THE INDEX ──` and the why when a stage starts, `↻ ready · kept from Oct 31` when one is skipped, and `+14 records` when a keep changes.
- **Results:** outputs grouped by stage; **Ask again** on stages whose inputs are independent of earlier stages.
- **Recipe card:** stage bands across the graph; the inspector shows the stage, its why, the keep's state and journal; a small Stages panel in the editor (add, rename, order, assign steps, keep on or off, refresh rule).
- **New recipe › A goal:** after Draft it, the strip is the first thing shown, with the graph folded.

One new kit component, `Strip`, in `@overtone-foundry/app-kit`.

## 6. The planner, which is the biggest change

The planner prompt and answer schema gain stages. What it is taught:

- A recipe is a strip of stages the user will read first; name them in the user's words, write a why for each, and put inputs where they are first needed.
- The three kinds of thing a recipe works on, and how to tell them apart: **material** (always the same; bake it into the recipe when the user's pasted material *is* the thing the recipe will work on), **inputs** (vary per run; make an input when the material is an example of what will vary), and a **run keep** (what runs remember). Say which in the why.
- When a stage gathers or prepares something that later runs will want again (an index, parsed pages, a transcript, a log of what has been seen), give it a keep, choose the refresh rule, and say in the why that it is remembered. Use the keep tools to write and read it.
- Stages are not steps: a stage holds a few steps that belong together; a recipe has two to six stages.
- Everything already taught stays: tools only from the list, no model or provider choice, names are word characters, references are dots.

The live tests from the first planner round are rerun with stages expected, plus three new goals: ask a folder of documents, a daily news log, a research brief that remembers its pages. The planner model is still a slot (local Auto; Codex by pin). Codex planning is tested separately because its answers differ in shape and verbosity; the fix loop and the checker are what make either acceptable.

## 7. Phases

| Phase | What ships | Sessions | Verified by |
|---|---|---|---|
| **1. Stages, the strip, the planner** | `stages` in the schema, `Recipe` and `Draft::check` rules, implied "Make" stage, the kit `Strip`, the strip on the four places, Set up grouped by stage, stage headings in the teletype, stage bands on the card, the four built-ins retrofitted with stages and whys. Planner taught stages and `why`; New recipe › A goal shows the strip first. | 3 | Tests for the rules and the implied stage; every built-in draws a strip; planned recipes arrive with stages (live, three goals); screenshots. |
| **2. Keeps and the journal** | `keep` on a stage with refresh rules, the recipe data folder, source fingerprints, the six keep tools, fresh/stale/not-built/records state words, Build now and Rebuild, kept outputs loaded for skipped stages, the journal in the inspector, Ask again in Results, the keep permission line. Two recipes prove it: a Daily News Log (links → new items → brief → the log grows) and Research Brief gaining a keep of parsed pages. | 3 | Engine tests: fresh skips, stale rebuilds, `always` runs and appends, manual waits, journal lines; live: the news recipe twice with `+N records` the second time, Research Brief refetching only a new link. |
| **3. Document tools and Ask Your Documents** | `doc.text` (PDF, DOCX, Markdown, text, with page numbers), `text.chunk`, `embed.texts` through an `embedding` slot wired into the Ollama adapter, `keep.search` by meaning over a searchable keep (brute-force cosine in SQLite), the Ask Your Documents recipe, Batch Briefing. | 3 | Tool tests on fixture files; live over a folder of ten documents, then Ask again twice without rebuilding; screenshots. |
| **4. Stage library** | Ready-made stages the planner may pick instead of composing from raw tools: each a named stage with its why, steps, prompts, and keep rule, built from the top twenty workflows in the competitor research (`references/popular-ai-workflow-templates.md` in the vault): read these links and remember them, index a folder, transcribe a recording, extract fields from a pile of files, draft in my voice, sort and file, repurpose a piece, translate in chunks, roll up spreadsheets, make study cards, and so on. The planner prompt lists them with one line each; a chosen stage is copied into the recipe (so the recipe stays self-contained and editable). | 3 | Each stage runs on its own fixture; the planner picks a library stage when the goal matches and composes when it does not (live, ten goals). |
| **5. Explain and tidy** | The inspector and the Recipe page lead with the stage and its why; a generated why when missing; stage-aware estimates ("stage ② about 4 minutes, once"); wording pass. | 1 | Jeremy's read-through of every recipe on screen. |
| **6. Retire and refit** | Inbox Triage moves to `samples/`; Learning Kit gets stages and a kept spine so re-making slides does not re-extract. | 1 | Tests, live run. |

Phase 1 starts now. Phases 2 and 3 are what Ask Your Documents, the news log, Recording to Notes (a kept transcript), and a remembering Research Brief all need; nested recipes and plug-in tools (MCP) come after and slot in without changing the stage model.

## 8. Decisions (confirmed 2026-10-31 unless marked)

- **Stages own their steps** (a list of step ids on the stage), not a `stage:` field on each step. Confirmed.
- **The strip is a second navigator under the Dial**, not a replacement. Confirmed.
- **Keeps live in Stillroom's data folder**, are deleted with the recipe, and never travel in a shared recipe. Confirmed.
- **Keeps are mutable and recorded**, not pure functions of their sources. A stage may read and change its own keep; every change is journaled; freshness is a per-keep refresh rule, not a purity rule. Replaces revision 1's "pure function" decision after Jeremy's answer.
- **One generic keep with store tools**, not a special vector-index tool pair. Search by meaning is a property of a keep. Replaces revision 1's `index.put`/`index.search`.
- **Search is brute-force cosine in SQLite first.** No vector database until a real folder is too big. Confirmed as the first version.
- **`why` is planner-written prose, editable, with a generated fallback.** Confirmed with a "think so"; revisit after phase 1 on screen.
- **Freshness rules** `when_sources_change`, `always`, `manual`; the planner picks, the card can change it. Confirmed.
- **The journal shows both**: a one-line "what changed" on the stage cell, linking to the full browsable history of records. Confirmed.
- **Material, input, run keep** are the three words for what a recipe works on, and Set up lets any input become material and back. Added after Jeremy's note that users will not realise a fixed document can be built into the recipe.
- **Models**: local only for now; users may pick any model later through the existing pins and provider settings. No Codex-specific work in this plan.
- **Retrofit the four built-ins in phase 1; Inbox Triage becomes a sample at the end.** Confirmed.
- **A stage library** is wanted, built from the top twenty competitor workflows; compose-from-tools ships first (phase 1), the library is phase 4. Confirmed.

## 9. Risks

- **Schema churn.** Stages and keeps touch the schema, the Rust types, the checker, the editor, the planner prompt, and four recipes. Mitigation: optional fields, implied stage, phase 1 before any store work.
- **The strip and the Dial compete.** Mitigation: the strip is smaller and lower contrast, only on recipe places; reviewed on screen in phase 1 before building on it.
- **Keeps grow without bound** (`always` logs). Mitigation: `keep.list` shows size in the strip; Rebuild empties; a per-keep cap can come later.
- **PDF text quality.** Mitigation: page-level chunks, a "no text layer" result instead of garbage, vision later.
- **No embedding model installed.** Mitigation: the strip says `needs an embedding model · pull one in Settings` on that stage; the Pull zone suggests one.

## 10. Status

- **2026-10-31, phase 1 slice 1 shipped** on the `stages` branch: the format, the checker, the implied stage, the four built-ins with whys, the planner writing stages (live: four stages on the first try), the kit `Strip`, status words from Set up and the run, the strip on Recipe, Set up, Running, and Results, inputs and result tabs grouped by stage, teletype stage headings, the Recipe page listing stages with whys, a planned recipe opening strip-first. Screenshots in `docs/screenshots/stages-*.png`. Remaining in phase 1: stage bands on the card and a Stages panel in the editor.
- **2026-10-31, phase 1 finished and phase 2 shipped** on the `stages` branch. Phase 1: stage bands with a label column on the card, a Stages panel in the editor. Phase 2: `keep` on a stage with `refresh: always | when_sources_change | manual`; a per-stage SQLite keep (records, manifest, journal) under `<app data>/recipe-data/<recipe>/<stage>/`; `keep.put`, `keep.get`, `keep.query`, `keep.list`, `keep.remove` reached through the tool context (own stage writes, earlier kept stages read by name); a fresh stage is skipped with its outputs loaded from the manifest and `reusedFrom: keep:<stage>`; source fingerprints over the inputs a stage reads (files by path, size, time) plus its step definitions and prompts; Rebuild empties and journals; shell commands `keep_states`, `rebuild_keep`, `keep_journal`, `keep_records`; strip words (`not built yet`, `fresh · N records · date`, `stale · will rebuild`, `N records · runs every time`, `ready · kept`); keep state and Rebuild in Set up; history (the journal) on the Recipe page; Ask again on Results; the checker refuses keep tools outside a kept stage; the planner taught keeps. Daily News Log is the proving recipe: two live runs, the second said "Nothing new since last time" with the keep at 2 records and a journal of `first build` then `runs every time`; Research Brief keeps what it read. The planner drafted a news-log recipe with `keep.query`/`keep.put` on the first try but forgot to declare the keep, which is what the new checker rule catches. Not built: `mode: append` as a separate flag (an `always` keep that reads itself covers it), `Build now` as a run that stops after a stage (Run builds it), journal lines in the teletype.
- **2026-10-31, phase 3 shipped** on the `stages` branch. Document tools: `doc.text` (PDF text layer via `pdf-extract`, DOCX via its `document.xml`, Markdown, text; pages for PDFs; a scan gives a note, not an error), `text.chunk` (overlapping passages ending at sentence breaks, page numbers kept), `keep.index` (embeds passages through an `embedding` slot and stores them with their vectors in the stage's keep; passages already stored unchanged are skipped) and `keep.search` (query embedded the same way, brute-force cosine over the keep, best passages with file name, path, page, score). Embeddings reach the model layer through `ModelClient::embed` (Ollama `/api/embed`), and tools reach models through the tool context (`slot` → resolved model). `fs.list` with an extension filter lists files only. Two built-ins: **Ask Your Documents** (folder → kept index → question → cited answer; live: first run indexed 38 passages in 2.4 s and answered PKCE with a citation; second run kept the index and answered a new question in 6.8 s; a new file re-indexed only its own passages) and **Batch Briefing** (folder, kept, plus links → one cited brief with a gaps section; live twice, the second run kept the files stage). Set up says when no installed model can do embeddings and which to pull; `nomic-embed-text` joined the starter list. The planner learns the two document shapes and that there is no `${keep}` variable; a drafted stage that writes a keep gets one declared. Not built: vision for scanned PDFs; Recording to Notes (needs local speech-to-text); `keep.search` with a filter.
- **2026-11-01, phase 4 built** on the `stages` branch: the stage library (`stages/<id>/stage.yaml` with prompts and schemas; thirteen stages: `index_folder`, `answer_from_index`, `read_folder`, `read_links`, `write_brief`, `summarize_file`, `extract_fields`, `rewrite_in_voice`, `repurpose_piece`, `study_cards`, `watch_pages`, `remember_seen`, `brief_new`; `requires` says which must come earlier; every stage expands into a recipe that checks clean). A used stage is copied into the recipe (inputs, slots, permissions, steps, outputs, files), so the recipe stays self-contained; the first stage that needs an input claims it. The planner became **two calls**: the strip first (`prompts/strip.md`, short, with the library listed tersely; ready-made stages by id in a constrained enum, custom ones by name and why), then the steps of the custom stages only, with the ready-made ones already expanded and their returns listed; all ready-made means one call. The single-call planner with a 24k-token prompt never wrote `use` even when its whys named the library stages, which is what forced the split. Live results of the four-goal test are recorded in the vault's [Stages](../jeremym-vault/Stillroom/projects/stages.md) note.
- **2026-11-01, phase 4 live**: documents Q&A, news log, and summarize-and-cards each planned in one call, 2–3 s, no problems, entirely from the library; the mixed interview-prep goal planned clean in 61 s after five merge rules (first stage keeps a shared input; optional sibling inputs `name?`; a ready-made stage id as a step id is rewritten; a ready-made stage named in a why counts as `use`; a repeated ready-made stage is dropped).
- **2026-11-01, phases 5 and 6 built** on the `stages` branch. Phase 5: Set up says what each stage took in past runs and whether it runs once (kept) or every run ("about 23 s, once"), with the total minus any kept stage that is fresh for the inputs typed; the step inspector leads with the step's stage and why; the Recipe page lists the stages first; a stage with no why gets one written from what it holds ("Takes folder. Runs list (a program) and answer (AI). Gives you answer."); the strip on Recipe and Set up reads the recipe and what is typed, never the last run; a keep says fresh or stale only once the inputs it reads are given, otherwise "built Nov 1"; a keep holding step outputs but no records never says "0 records"; a keep-served step says "kept". Phase 6: Inbox Triage moved to `samples/inbox-triage` (six built-ins remain; it still reads clean in the tests); Learning Kit's spine stage is a run keep (`when_sources_change`). Live: Learning Kit four times through the harness; the first three rebuilt the spine because the harness wrote its sample to a new temp folder each run (a changed path is a changed source, as designed); with a fixed path the fourth run served `read` and `extract` from the keep in 0 ms and finished in 95 s against 147 s. Screenshots `docs/screenshots/explain-*.png`. Not built: a per-stage estimate in the strip cells (Set up has it); stage bands for a stage with only inputs (the card draws bands by step rows).

