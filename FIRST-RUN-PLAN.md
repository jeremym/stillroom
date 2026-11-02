# The first run is a goal

Status: proposal, revision 1, 2026-11-02, from Jeremy's from-scratch install of 0.3.2. Read `FIRST-USE-PLAN.md` for the goal screen and the planner as they are now, `BROWSING-PLAN.md` for the shelf and the catalog, and `PLAN.md` §11 and §12 for what the product refuses to do (send, post, hold credentials) and what is roadmap. This plan replaces the welcome wizard with one screen where the first thing a person does is say what they want.

## 0. Vocabulary

| Word | Means |
|---|---|
| **First run** | The screen a person sees once, on first launch, instead of the Library. |
| **Example goal** | One of a handful of full sentences a person can click to fill the goal box, each of which the planner is known to draft clean. |
| **Plumbing** | Ollama running, a model that can plan, where results go. Needed, but not what the person came for. |
| **Deliver** | The last stage of a recipe, where the result is written somewhere the person can reach it: a folder of their choosing, their notes app's folder, the run's own results folder. |

## 1. In one screen

**The problem.** The welcome is three screens about the machine (where the thinking happens, how to choose a model, a document to try) and ends by setting up a run of a document recipe. It keeps saying "document" because the only thing it knows how to offer is the one built-in on the shelf. A person who opened Stillroom wanting a morning news brief or a mail sorter sees nothing about either.

**The proposal.** The first run is the goal screen, with the plumbing folded underneath it:

```
 ┌───────────────────────────────────────────────────────────────────────────┐
 │  STILLROOM                                                                 │
 │  Describe the goal. Get the result. See how it was made.                   │
 │                                                                            │
 │  WHAT SHOULD IT DO? *   the one thing you must fill in                     │
 │  ┌────────────────────────────────────────────────────────────────────┐   │
 │  │ Say in plain words what you want it to do…                          │   │
 │  └────────────────────────────────────────────────────────────────────┘   │
 │  Or start from one of these:                                               │
 │   · Each morning, read the sites I follow about a topic, keep only what    │
 │     is new, and write me a short brief into my notes folder.               │
 │   · When a saved email lands in a folder, sort it, file it, and draft a    │
 │     reply for me to send.                                                  │
 │   · Ask me for a folder of PDFs and a question, and answer from them with  │
 │     the page it came from.                                                 │
 │   · Turn a document into short notes, study cards, and a quiz.             │
 │   · Read the links I give it about a question and write a cited brief.     │
 │                                                                            │
 │  [ DRAFT IT ]   or browse the catalog · skip for now                       │
 │                                                                            │
 │  ● Ollama · 4 models · qwen3.8:27b will plan (fits in memory)   change     │
 └───────────────────────────────────────────────────────────────────────────┘
```

The planner drafts the first recipe while the stream window shows what it is writing; the recipe opens on its page with the strip; the shelf now holds it. The Ollama and model checks become one status line at the bottom, which only unfolds (into the existing step-1 content: install Ollama, pull a starter model) when something is missing. Preference and defaults go to Settings, where they already live.

## 2. The example goals, and what the engine can honestly do

Each example is a full goal sentence the planner drafts clean today, or will after §3. The wording is chosen so that nothing promised is impossible.

| Example (as shown) | What it needs | Today | What changes |
|---|---|---|---|
| Each morning, read the sites I follow about a topic, keep only what is new, and write me a short brief into my notes folder. | fetch links, a log of what was seen, a brief, write a file under a chosen folder | Reading, remembering, and the brief: yes (Daily News Log). Writing into a chosen folder: yes (`fs.write` under a `directory` input, FIRST-USE §4 rule 9). **"Each morning": no.** Nothing runs on a clock; watches fire on folder changes, while the app is open. | §3.1 schedules. Until then the example reads "Read the sites I follow…" without "each morning", and the Recipe page says "Run it each morning, or set a schedule once Stillroom has them". |
| When a saved email lands in a folder, sort it, file it, and draft a reply for me to send. | watch a folder, parse `.eml`, classify, file, write a draft | Yes, exactly the Inbox Triage sample: a folder watch, `email.parse`, an agent that files, `email.write_draft`. Stillroom never sends. | Move Inbox Triage from samples into the catalog proper once its fixtures are not needed (it needs an inbox folder of `.eml` files, which Mail.app can export). A one-page how-to: a Mail rule that saves matching messages to a folder, which the watch picks up. |
| Ask me for a folder of PDFs and a question, and answer from them with the page it came from. | index a folder, search by meaning, cite pages | Yes (Ask Your Documents). | none |
| Turn a document into short notes, study cards, and a quiz. | read a file, write notes, cards, quiz | Yes (Learning Kit, `study_cards`). | none |
| Read the links I give it about a question and write a cited brief. | fetch links, extract with quotes, brief | Yes (Research Brief). | none |

Not offered, and said plainly on the page under a folded "What Stillroom cannot do yet": **send mail or messages**, **post to Notion, Slack, or any service** (nothing leaves this Mac; a recipe writes files, and your notes app can watch a folder), **read a mailbox directly** (save messages to a folder with a Mail rule instead), **transcribe a recording** (no local speech-to-text yet), **run on a schedule** (until §3.1). This is the same honesty the catalog's "may touch" line already has, applied to the first screen.

Notion specifically: Notion's desktop app can import Markdown; its API needs a token and a network call, which the product refuses by default (PLAN §11). The honest deliver target is a folder; a person who wants Notion points the recipe at a folder Notion imports from, or at their Obsidian vault.

## 3. What the engine needs

### 3.1 Schedules (the one real gap)

A schedule is a saved binding like a watch (STAGES-PLAN §8 decision: a trigger is a binding, not part of the recipe): recipe, inputs, pins, and a time of day with the days it applies. While the app is open, the runtime fires it at that time (one run at a time, through the existing run gate, carrying `trigger: { kind: "schedule" }` so history says "by a schedule"). If the app was closed at the time, it fires once on the next launch if the last run is older than a day ("caught up"). Settings › Watches becomes Settings › Watches and schedules; Set up gains **Run every day at…** beside **Watch a folder**. The strip's first stage says "runs every morning at 7". Running while the app is closed stays roadmap (menu-bar mode), stated on the page.

Rust: `store` table `schedules`, `src-tauri/src/schedules.rs` next to `watches.rs`, a timer task. Frontend: `SchedulePanel` beside `WatchPanel`, the Set up button, the history word. Two sessions. Verified by a schedule set two minutes ahead firing live and a caught-up run on relaunch.

### 3.2 The planner says what it cannot do

A rule in `prompts/strip.md` and `prompts/planner.md`: when the goal asks for something no tool does (send, post to a service, run on a clock, read a mailbox, transcribe), do not invent a step; make the last stage write a file to a folder the user chooses and say in that stage's why what was asked for and what the recipe does instead ("Notion has no door here; the brief is saved as Markdown in the folder you choose, which Notion can import"). The checker already refuses unknown tools; this makes the model's fallback a deliberate one. Half a session, proved on the news-to-Notion goal headless.

### 3.3 Either-or sources

Backlog item from FIRST-USE §9: a loop over an empty optional input is an error. Fixing it (an empty `over` runs zero times; a stage whose only inputs are empty is skipped with the strip word "nothing given") lets the planner take "links or a folder, whichever you give" instead of asking for both. One session. Needed for the first example to feel right when a person has only links.

## 4. The first-run screen

- A new place `first-run` shown when `firstRun` is unset, replacing `Welcome`. It is `NewRecipe` in goal mode with three differences: the example list under the goal box (clicking one fills the box, including the "Built into the recipe" field when the example has material), the plumbing status line at the bottom that unfolds into the existing Ollama and model-pull content when `provider.available` is false or no model can plan, and the footer links **browse the catalog** (lands on the Catalog with the welcome marked done) and **skip for now** (Library, one recipe on the shelf).
- Draft it with nothing installed is disabled with "needs a model that can plan"; the line below says which to pull and does it in place.
- After the plan: the Recipe page of the new recipe, `firstRun` done; the note says "Your first recipe. Set up a run, or open the recipe card to see how it was made."
- The example goals live in one file, `src/firstRun/examples.ts`, each with its sentence, optional material, and the catalog recipe it most resembles (so "see a finished one" can open that recipe's Look page). Each example is also a planner-answer fixture in `crates/core/tests/compat/planner-answers/first-run-*.json`, recorded from a clean live draft, so a prompt change that breaks an example fails a test.
- The kit needs nothing new; the existing `Zone`, `Pill`, and `PullModel` cover it.

## 5. Phases

| Phase | What ships | Sessions | Verified by |
|---|---|---|---|
| **1. Goal-first first run** | The `first-run` place, the five examples (wording as in §2, "each morning" left out), the plumbing line, skip and browse, `Welcome.tsx` retired. | 1 | A fresh data folder boots to the goal screen; each example drafted clean live on the suggested model; screenshots. |
| **2. Honest planner** | §3.2 rule, the "cannot do yet" fold, the Mail-rule how-to in `docs/`, Inbox Triage promoted when its fixtures are documented. | 1 | The news-to-Notion goal drafts a folder deliver stage whose why says so; fixtures. |
| **3. Schedules** | §3.1. The first example regains "each morning". | 2 | Live timer fire and caught-up run; screenshots. |
| **4. Either-or sources** | §3.3. | 1 | A plan with only links given runs with the folder stage skipped; strip word. |

Phase 1 is the one that changes the first minute; 2 keeps the promises honest; 3 and 4 make the first example true end to end.

## 6. Decisions to confirm

- **The welcome wizard goes; the first run is the goal screen with examples.** Proposed.
- **Examples promise only what the engine does.** "Each morning" waits for schedules; "Notion" is a folder the person points at. Proposed.
- **Schedules run while the app is open, with a catch-up on launch**; a menu-bar mode is still later. Proposed.
- **Mail stays folder-based** (a Mail rule saves messages to a folder; Stillroom never reads a mailbox or sends). This is the existing principle, restated.
- **Inbox Triage moves from samples into the catalog** once the how-to exists. Proposed.

## 7. Risks

- **A four-minute first plan.** The stream window and the measured model help; the example goals are the ones that plan fastest (two to four stages, mostly ready-made). If the suggested starter model plans the examples in under a minute, say so on the screen ("about a minute on this Mac").
- **An example that stops drafting clean** after a prompt or library change: the fixtures catch it in CI, not on a stranger's first run.
- **Schedules while closed** will be asked for the moment schedules exist; the page says "while Stillroom is open" in the same breath.
