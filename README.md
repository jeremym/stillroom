# Stillroom

A goal-first, local-first AI workflow studio for people who have never built a workflow.

> Describe the goal. Get the result. Open the door.

Describe what you want to accomplish. Stillroom runs a **Recipe** (a portable folder of steps, prompts, and schemas) against local models and hands you real outputs: notes, slides, diagrams, audio, tables, briefs, filed email. Then, only if you want, open the door and see the machine that made them: the graph, the prompts, the models, the data flowing between steps. Every recipe can be inspected, edited on the graph, rerun, and shared.

Local models by default. No Docker, no account, no mandatory cloud. Optional, off until you turn them on: the Codex app as a cloud provider on your ChatGPT plan, and any OpenAI-compatible service by API key.

## Status

Pre-alpha, macOS, developed in the open on a private repository. What works today:

- **Four built-in recipes.** Summarize a File, Learning Kit (notes, flashcards, concept map, process diagram, slides, narrated audio from one document), Inbox Triage (sort saved emails, file them with an agent, draft replies, approve before anything is written), Research Brief (fetch pages, extract quoted findings, write a cited brief).
- **An engine that is general.** Tool, model, agent, map, and review steps; model slots resolved from what is installed (pin, recommendation, or capability); schema-constrained outputs; filesystem and network permissions enforced per recipe; selective rerun from a step cache; edit any artifact and only downstream steps regenerate; continuing a run never repeats a side effect.
- **Flow View.** The recipe as a readable top-down map with the run's state on it, and an edit mode with forms per step, drag-to-wire, live checks from the runtime, and save as a copy or in place.
- **Folder watches.** Point a recipe at a folder and it runs whenever something lands there, while the app is open. Each run continues the last, so files handled before are not handled again and reviews only ask about what is new.

Not yet: a planner that drafts a recipe from a goal, sharing, MCP and Agent Skills, voices other than the system one. The API-key provider is verified only against a local OpenAI-compatible server. The design and roadmap are in [PLAN.md](PLAN.md).

## Layout

```
schemas/        Recipe format v0 (JSON Schema)
recipes/        Built-in recipes: recipe.yaml, prompts/, schemas/
samples/        Fixtures for Inbox Triage (copy them before running against them)
crates/core     Recipe types, executor, tools, step cache, propagation, draft editing, folder watcher, SQLite store
crates/models   Provider trait, Ollama native adapter, Codex CLI adapter, OpenAI-compatible adapter, router; live tests under tests/
src-tauri/      Tauri 2 desktop shell and commands (Rust)
src/            React + TypeScript frontend; src/flow is the graph view and editor; the look comes from @overtone-foundry/app-kit (../overtone-foundry-app-kit, linked), src/brand.tsx holds Stillroom's own marks
```

User recipes live in the app data folder under `recipes/` and shadow a built-in with the same id. Runs, the step cache, and watches live in `stillroom.db` next to them.

## Install

Apple silicon Mac, macOS 13 or newer, and [Ollama](https://ollama.com) with one chat model.

```
brew install --cask jeremym/tap/stillroom
open -a Stillroom
```

Or download `Stillroom-X.Y.Z.zip` from [Releases](https://github.com/jeremym/stillroom/releases), unzip, move to Applications, and right-click → Open the first time.

Stillroom is signed with the project's own certificate, not an Apple Developer ID, and is not notarized. The cask removes the quarantine flag on install so it launches without a Gatekeeper prompt.

### Verifying a download

```
shasum -a 256 Stillroom-X.Y.Z.zip             # matches the release .sha256
codesign -d -r- /Applications/Stillroom.app   # check the leaf hash below
```

A genuine build prints `certificate leaf = H"<leaf hash>"` (recorded here once the release certificate exists).

### Releasing (maintainer)

Once, attended: `scripts/create-release-cert.sh` (two keychain prompts; back up the .p12). Then per release: bump the version in `package.json`, `src-tauri/tauri.conf.json`, `Cargo.toml` and `src-tauri/Cargo.toml`, run `scripts/build-release.sh --version X.Y.Z`, create the GitHub release with the files it names, and `scripts/update-tap.sh` to bump the cask in `jeremym/homebrew-tap`. The cask itself lives in `distribution/stillroom.rb`.

## Development

Requires Rust 1.90+, Node 22+, pnpm, Xcode command line tools, and [Ollama](https://ollama.com) with at least one chat model for anything that calls a model.

```
pnpm install
pnpm tauri dev          # the app
pnpm test               # frontend tests
cargo test --workspace  # Rust tests
```

Live runs without the UI: `crates/models/tests/ollama_live.rs` (ignored by default; driven by `STILLROOM_*` environment variables).
