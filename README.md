# Stillroom

A goal-first, local-first AI workflow studio for people who have never built a workflow.

> Describe the goal. Get the result. Open the door.

Describe what you want to accomplish. Stillroom runs a **Recipe** (a portable folder of steps, prompts, and schemas) against local models and hands you real outputs: notes, slides, diagrams, audio, tables, briefs, filed email. Then, only if you want, open the door and see the machine that made them: the graph, the prompts, the models, the data flowing between steps. Every recipe can be inspected, edited on the graph, rerun, and shared.

Local models by default. No Docker, no account, no mandatory cloud.

## Status

Pre-alpha, macOS, developed in the open on a private repository. What works today:

- **Four built-in recipes.** Summarize a File, Learning Kit (notes, flashcards, concept map, process diagram, slides, narrated audio from one document), Inbox Triage (sort saved emails, file them with an agent, draft replies, approve before anything is written), Research Brief (fetch pages, extract quoted findings, write a cited brief).
- **An engine that is general.** Tool, model, agent, map, and review steps; model slots resolved from what is installed (pin, recommendation, or capability); schema-constrained outputs; filesystem and network permissions enforced per recipe; selective rerun from a step cache; edit any artifact and only downstream steps regenerate; continuing a run never repeats a side effect.
- **Flow View.** The recipe as a readable top-down map with the run's state on it, and an edit mode with forms per step, drag-to-wire, live checks from the runtime, and save as a copy or in place.
- **Folder watches.** Point a recipe at a folder and it runs whenever something lands there, while the app is open. Each run continues the last, so files handled before are not handled again and reviews only ask about what is new.

Not yet: a planner that drafts a recipe from a goal, sharing, MCP and Agent Skills, cloud providers, voices other than the system one. The design and roadmap are in [PLAN.md](PLAN.md).

## Layout

```
schemas/        Recipe format v0 (JSON Schema)
recipes/        Built-in recipes: recipe.yaml, prompts/, schemas/
samples/        Fixtures for Inbox Triage (copy them before running against them)
crates/core     Recipe types, executor, tools, step cache, propagation, draft editing, folder watcher, SQLite store
crates/models   Provider trait, Ollama native adapter, router; live test harness under tests/
src-tauri/      Tauri 2 desktop shell and commands (Rust)
src/            React + TypeScript frontend; src/flow is the graph view and editor
```

User recipes live in the app data folder under `recipes/` and shadow a built-in with the same id. Runs, the step cache, and watches live in `stillroom.db` next to them.

## Development

Requires Rust 1.90+, Node 22+, pnpm, Xcode command line tools, and [Ollama](https://ollama.com) with at least one chat model for anything that calls a model.

```
pnpm install
pnpm tauri dev          # the app
pnpm test               # frontend tests
cargo test --workspace  # Rust tests
```

Live runs without the UI: `crates/models/tests/ollama_live.rs` (ignored by default; driven by `STILLROOM_*` environment variables).
