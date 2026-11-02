# Browsing: the shelf, the catalog, and a format that outlives them

Status: proposal, revision 1, 2026-11-02, written after the review of the `stages` branch (findings in the vault: `Stillroom/references/review-2026-11-02.md`). Read `PLAN.md` for the product, `UI-PLAN.md` for the look, and `STAGES-PLAN.md` for stages and keeps. This plan does three things in order: fixes what the review found, gives the recipe format a way to change without breaking recipes, and then makes recipes browsable: one example on the shelf at install, a catalog to look through, and an archive.

## 0. Vocabulary (the words to correct me with)

| Word | Means | Not to be confused with |
|---|---|---|
| **Shelf** | The recipes you have chosen to keep in front of you. What the Library's Recipes zone shows. Starts with one recipe. | **Catalog**: everything you could put on the shelf. |
| **Catalog** | Every recipe that ships with the app, in two rows: **built-in** (written here) and **community** (contributed through the repository, reviewed by Jeremy before it ships). Browsed, not installed from the internet. | A registry or hub (decided against: supply-chain risk). |
| **Example** | The one recipe on a fresh shelf: *Describe a document*. Two steps, one input, a two-sentence description. | The Learning Kit showcase, which stays in the catalog. |
| **Archive** | Where a shelved recipe goes when you take it off the shelf without deleting it. Folded at the bottom of the Library. Restore puts it back. | Delete, which removes a recipe you own and its run keeps. |
| **Provenance** | Where a recipe or a stage came from: catalog id and version, or a ready-made stage id, version, and a digest of what was copied. Lets the app say "a newer version exists" and never overwrite your edits. | |
| **Migration** | A function that rewrites an older recipe's YAML into the current shape before anything else reads it. One per version step, tested on frozen fixtures. | |
| **Note** | A checker finding that does not stop a save: "this recipe uses X, which is now Y. Fix it." | **Problem**: a finding that does stop a save. |

## 1. In one screen

**What the review found.** Nothing in the stages work broke *parsing*: `stages` is optional and every older recipe still loads. What broke was the *contract*: the checker gained refusal rules (a keep tool needs a kept stage, a step may not read a later stage, one item in one stage) and `Draft::write` refuses on any problem, so planner-written drafts and hand-edited copies that were fine the day before were refused, and the planner needed five merge rules and a two-call split to stop producing refusable answers. The format has nine copies of its shape (schema, Rust types, checker, `RecipeInfo`, stage library, planner answer schema, planner prompt, SQLite tables, TypeScript mirrors), no frozen fixtures, no `minVersion` enforcement, and no way to say "this is old, here is the fix" instead of "no".

**The proposal, in three parts.**

1. **Fix first** (§2): the three executor bugs, the deleted-recipe keeps, the DOCX inflation, the content policy of the webview, and the three unscoped file commands. Before `stages` merges.
2. **A format layer** (§3): frozen fixtures, a `migrate` chain keyed on `schemaVersion`, a deprecation registry whose entries double as checker Notes with a one-click Fix, `runtime.minVersion` enforced and stamped, an `x-` namespace for experiments, versioned ready-made stages with provenance, and SQLite `user_version` ladders. This is the "go-between and deprecation machine" Jeremy asked for, and it is what would have turned this phase's breakage into a Note and a test failure.
3. **Browsing** (§4, §5): the shelf with one example recipe, the catalog with a page per recipe that says what it does, needs, gives, and may touch, Add to shelf, the archive, a community row in the repository with a review checklist and a protected `main`, and a folder-or-zip import with a consent step.

```
 LIBRARY
 ┌ Recipes · 1 on the shelf ───────────────── Browse the catalog · New recipe ┐
 │  📄 Describe a document                                                   │
 │     Say what a file is and what is in it, in a few sentences.             │
 │     2 steps · a file · yours                                              │
 └───────────────────────────────────────────────────────────────────────────┘
 ▸ Archive (0)
 Previous runs · Watches · Usage

 CATALOG                                 all · documents · research · news · learning
 ┌ Ask Your Documents ──────── ┐ ┌ Research Brief ─────────── ┐ ┌ Daily News Log ───────── ┐
 │ Index a folder once, then   │ │ Read links, quote them,    │ │ Log only what is new on  │
 │ ask it anything, cited.     │ │ write a cited brief.       │ │ the pages you follow.    │
 │ needs a folder · keeps an   │ │ needs links · reaches your │ │ needs links · keeps a    │
 │ index · runs on what I have │ │ links only                 │ │ log · runs every time    │
 │ [Add to shelf]  [Look]      │ │ [Add to shelf]  [Look]     │ │ [Add to shelf]  [Look]   │
 └─────────────────────────────┘ └────────────────────────────┘ └──────────────────────────┘
```

## 2. Fix first (before `stages` merges)

**Status 2026-11-02: shipped on `stages`** (commits fb30d53 … 145bbab). Live: the app boots with the Rust-built windows, Settings opens, a result's remote image renders as text, a link opens the browser while the window stays put, the Rust save dialog cancels and saves (`docs/screenshots/phase0-*.png`); 78 Rust and 80 frontend tests. Also found and fixed on the way: `crates/models` had never been committed (the `models/` ignore line matched it), so no clone could build; the `.gitignore` is now `/models/`. Rulesets protecting the default branch were added to all ten public repositories.

From the review. Each is a confirmed defect with a reproduction or a verified code path. File and line are as of `35911a8`.

| # | Defect | Where | Fix |
|---|---|---|---|
| F1 | A step held behind a review inside a kept stage is journaled as built with a null output; the next run serves null from the keep and fails until Rebuild. | `executor.rs:579`, `:763` | Do not write a manifest while any step of the stage was held or `needs_review` is set; a manifest holding a held output is "not built". |
| F2 | Edit-and-rerun ignores the keep: an edited spine is replaced by the kept one and downstream regenerates from the old spine. | `executor.rs:534` | A step named in `req.edits` never takes the kept path; the stage reruns with the edit on top and rewrites the manifest. |
| F3 | A keep reader declared before its writer runs first (no data dependency from `keep.get/query/search` to the kept stage). | `executor.rs:414`, `draft.rs:186` | Implicit dependency: a step reading keep X depends on every step of X; the checker refuses reading a later stage's keep. |
| F4 | Deleting a recipe leaves `recipe-data/<id>/`, although Set up promises it is deleted. | `lib.rs:837` | `remove_dir_all` the keep folder on delete; on save, drop keep folders of stage ids no longer present (or list them as orphans). |
| F5 | Passages of deleted or shortened files stay in an index forever; `remove` leaves the vector row. | `tools.rs:300`, `keep.rs:174` | After indexing a source, delete its records past the new chunk count; at stage end remove records whose source is gone; `remove` deletes vectors too. |
| F6 | Changing the embedding model leaves the index "fresh"; queries embedded by one model are compared with vectors from another. | `executor.rs:144`, `keep.rs:185` | Resolved models of the stage's slots join the fingerprint; `keep.index` skips a passage only when `vectors.model` matches. |
| F7 | DOCX inflation is unbounded: a 98 KB file took the process to 704 MB; a 100 MB file would kill the app. | `tools.rs:107` | Check `ZipFile::size()` and read through `.take(32 MiB)`; strip tags in one pass. |
| F8 | No content policy; model Markdown can load remote images on render (a silent leak in a local-first app); result links navigate the window away. | `tauri.conf.json:23`, `viewers/index.tsx:31`, no `on_navigation` | Set a CSP (self, data, blob, asset, ipc only); a Markdown `img` that renders remote sources as text; `on_navigation` allowing only the app origin, links opened by the opener after a click-through. |
| F9 | `write_text_file`, `write_binary_file`, `copy_file` take raw paths from the webview. | `lib.rs:1035`, `:1041`, `:1055` | The save dialog moves to Rust and the write uses the path it returned; `copy_file` resolves the source from the run's artifact record. |
| F10 | Stop shares one cancel slot between a run and a plan; `rebuild_keep`/`keep_journal`/`keep_records` join raw `stage` strings into a path; `opener:default` is granted but unused by the frontend. | `lib.rs:227`, `:691`, `:770`, `capabilities/default.json:14` | Separate cancels; validate `stage` against the recipe's stages; drop `opener:default`. |

Tests to add with them: a review-inside-a-kept-stage run, an edit of a kept step, a reader-before-writer recipe that the checker now refuses, a delete that removes keeps, an index that forgets a removed file, a DOCX over the inflated cap, and a `tsc` + Vitest check that the Markdown viewer renders a remote image as text.

Carried into the phases below rather than fixed first: permission consent for literal paths and hosts (§5 phase 4), planner-written permissions whitelisted to `input`, `output`, `${inputs.x}` and what the library stage brought (§5 phase 1), symlinks inside a bundle (phase 4, with import), the Codex wording and warning (phase 4), loopback and LAN refusal in `http.fetch` unless named literally (phase 4), the watch-fires-mid-run hijack (phase 2, with job ids), the library slot merge keeping the first `requires` (phase 1, union them).

## 3. The format layer

**Status 2026-11-02: shipped on `stages`** (merge aeb1bf8). Live: a recipe with `minVersion: 9.0.0` reads "too-new could not be opened. This recipe needs Stillroom 9.0.0 or newer; you have 0.2.0." in the Library; a recipe whose stage writes a keep without declaring one shows "Worth a look · Stage “Remember” writes a keep but does not say so · Declare the keep" on its card, and the fix clears it (`docs/screenshots/phase1-*.png`). 97 Rust tests including the compat fixtures (`crates/core/tests/compat/`), 129 frontend. One deviation from the text below: the version floor stamped on save is capped at what this build supports (`migrate::supported_version`), so a 0.2.0 build does not refuse its own 0.3.0-floor saves; revisit when 0.3.0 is cut.

The rule stays: the Rust side is the only validator; the UI renders what Rust returns. Everything here lives in `crates/core`.

### 3.1 Fixtures, before any code

`crates/core/tests/compat/`:

- `recipes/v0-pre-stages/`: `learning-kit`, `research-brief`, `summarize-file`, `inbox-triage` as they were on `main` before stages (`git show main:recipes/...`).
- `recipes/v0-stages/`: today's six built-ins and `samples/inbox-triage`.
- `planner-answers/`: the live answers from the four-goal test, including the ones that needed merge rules, as JSON.

Two tests. Every fixture loads through `RecipeBundle::load`, checks with no Problems (Notes allowed), and `migrate` is idempotent on it. Every property in the planner's `answer_schema()` and `strip_schema()` exists in the recipe schema. From now on, a format change is a change to these folders first.

### 3.2 `migrate.rs`

`pub fn migrate(yaml: Value) -> Result<Migrated, MigrateError>` with `Migrated { yaml, from_version, notes }`. A chain of `fn v0_to_v1(&mut Value) -> Vec<Note>` steps. Called from `Recipe::from_yaml`, `Draft::read`, and the planner's `parse_answer`, **before** schema validation; the loaded recipe is then checked against the current schema only. `schemaVersion` becomes an enum of accepted strings (stays a string: YAML would read a bare `1` as a number). `Draft::write` stamps the current version.

### 3.3 The deprecation registry, and Notes with a Fix

One table in `migrate.rs`:

```rust
struct Deprecation { path: &'static str, since: &'static str, replaced_by: &'static str, rewrite: fn(&mut Value) -> bool }
```

`migrate` applies rewrites when it bumps a version. `Draft::check` consults the same table when the version is current and returns a **Note**, not a Problem. `check` returns `Vec<Finding { text, severity: Problem | Note, fix: Option<FixId> }>`; a new `Draft::fix(&[FixId])` applies the named rewrites; `Draft::write` refuses only on Problems. The first entry is the keep-declaration repair that today lives only in the planner (`declare_keeps`): a stage whose steps write a keep gets one declared. Moving it here means the editor's Problems pane, the loader, and the planner all offer the same repair.

Shell: `check_recipe_draft` returns findings; new `fix_recipe_draft`. UI: the Problems list shows Notes in the quiet style with a **Fix it** button; wording "This recipe uses `x`, which is now `y`."

### 3.4 `runtime.minVersion`, enforced and stamped

`Recipe` gains `runtime: Option<Runtime>` (it is in the schema today and read nowhere). `RecipeBundle::load` compares to the app version and returns `BundleError::NeedsNewer`, so the Library reads "This recipe needs Stillroom 0.5 or newer; you have 0.4" instead of a schema message. `Draft::write` stamps `minVersion` from a feature table (uses `stages` → 0.3, uses `keep` → 0.3, …), so a shared recipe carries its floor without the author knowing.

### 3.5 `x-` for experiments

`patternProperties: { "^x-": {} }` at the top level and inside step, stage, input, slot, and output. An experimental sub-object rides along unparsed, round-trips through the editor (the TypeScript side already keeps unknown keys), and is promoted by moving it out of `x-` with a registry rewrite. This is where a new object goes while it is being designed, so the next "stages" never has to be right on the first commit.

### 3.6 Versioned ready-made stages, with provenance

`stage.yaml` gains `version: 1`. `expand_into` writes on the stage entry it creates:

```yaml
source: { stage: index_folder, version: 1, digest: "sha256:…" }   # digest of the copied steps
```

`check` emits a Note "a newer version of the ready-made stage *Index a folder* exists. Update it" only when the digest still matches the copied steps; edited copies are never touched. `keep` gains an optional `id` (default: the stage id) so a stage can be renamed without orphaning its records. The planner's `answer_schema()` is derived from the recipe schema's `properties` rather than hand-copied, and `PlanOutcome` records which schema version the answer was written against. When two library stages declare the same slot, `requires` is the union.

### 3.7 SQLite `user_version`

`store.rs` and `keep.rs`: a `PRAGMA user_version` ladder, one `ALTER` or backfill per step. The keep's `meta` table gains `format` (the fingerprint scheme version); when it differs the strip reads "rebuilt because Stillroom changed how it checks sources", not "stale". Built-in `version` is bumped whenever `stages`, `keep`, or `source` change, and the Library marks a shadowed built-in whose version is newer than the user's copy.

## 4. Browsing

### 4.1 The shelf

**Status 2026-11-02: shipped on `stages`** (merge d1ff315). Live: the existing database climbed to store version 2 and seeded the shelf with the example plus the three built-ins that had runs; Archive moved Learning Kit into a folded Archive zone and Restore brought it back; the Catalog zone (folded, phase 3 makes it a place) added Research Brief to the shelf; the card menu shows Delete and Reveal only for your own recipes (`docs/screenshots/phase2-*.png`). 101 Rust tests, 184 frontend. The watch-mid-run fix shipped with it: `run:started`, `runId` on every run event, a watch says started only once it holds the run gate.

What the Library's Recipes zone shows. A shelf entry is either a recipe in the user's folder (`yours`) or a catalog id the user put on the shelf (`built-in` or `community`, by reference, never copied). The list of shelved catalog ids and the archived ids live in the store (`shelf` table: `recipe_id, kind, shelved_at, archived_at`), not in the recipe, so a shared recipe never carries anyone's shelf.

- **Fresh install:** one entry, the example recipe *Describe a document* (`recipes/describe-document`: read a file, write two or three sentences about what it is and what is in it; icon `notes`; description under 140 characters so the card stays short). Welcome's last screen opens its Set up, as today.
- **Card:** glyph, name, the one-line description, then one row of tags: step count, the required inputs, `yours` / `built-in` / `community`, `needs a model` when nothing installed fits, last run. Descriptions are clamped to two lines on the card; the full text is on the Recipe page.
- **Card menu (⋯):** Set up a run · Recipe card · Duplicate · **Archive** · Delete (yours only, with "also deletes what it kept") · Reveal in Finder (yours only).
- **Empty shelf:** "Nothing on the shelf. Browse the catalog, or make a recipe."
- **Archive:** a folded zone under Recipes, "Archive (n)". Archived recipes keep their runs, watches (paused), and keeps; the card shows **Restore**. Archiving a catalog recipe is just leaving the shelf; archiving one of yours hides it. Nothing is ever deleted by archiving.

### 4.2 The catalog

A new place, `browse` (⌘B), reached from a **Browse the catalog** button beside New recipe and from the empty-shelf line. The Rail breadcrumb reads `Library › Catalog`.

- **Filter row:** `all · documents · research · news · learning · text · samples`, drawn from the recipes' `tags`; a toggle **runs on what I have** (every slot fillable by an installed model); a search box over name, description, and whys.
- **Card:** as on the shelf, plus two lines that answer "what is this": **needs** (inputs by type: a folder, links, a file) and **may touch** (the existing permission sentence: reads the files you choose, reaches only your links, keeps an index on this Mac, no network). Buttons **Add to shelf** and **Look**. A card already on the shelf says `on the shelf`.
- **Look** opens the existing Recipe page in read-only form: description, needs, gives you, thinks with, may touch, the stages with their whys and steps, and, when the recipe ships one, **See an example result** (§4.4). The primary button is **Add to shelf** (then **Set up a run** appears). Nothing runs from the catalog without being shelved first, so the shelf always tells the truth about what the user has used.
- **Rows:** built-in first, then community (with `author.name` and "reviewed before it shipped" in the card's provenance line), then samples with "needs the sample folders; copy them first".
- **Updates:** a shelved catalog recipe is always the shipped version (by reference). A *duplicated* one carries `source: { catalog: id, version }`; when the catalog's version is newer and the user's steps still match the digest, its card says "a newer built-in exists · Compare".

### 4.3 The repository side

Recipes keep living in the public repository, which is how "community" works without a registry:

- `recipes/` stays the built-in row. A new `community/<id>/` folder is the community row; both are bundled into the app as resources and scanned as catalog roots (`RecipeSource` gains `Community`).
- `CONTRIBUTING.md` with the review checklist a community recipe must pass: `check` clean with no Notes, permissions limited to `input`, `output`, and literal hosts the recipe needs, no `shell`, no `cloudModels`, prompts read for injection, `author` filled in, `minVersion` stamped, and one live run by the maintainer before merge.
- `main` gets a ruleset: pull requests required, force-pushes and deletions blocked, the owner may bypass for releases. Today the repository is public, you are the only collaborator (so nobody else can push), and `main` has no protection. A ruleset is a GitHub change, so it is yours to say yes to; the command is one `gh api` call and is in §7.
- `.github/CODEOWNERS` naming you for `recipes/`, `community/`, `stages/`, `schemas/`, and `prompts/`.

### 4.4 Understanding a recipe before running it

Two additions so a reader never has to open YAML:

- **See an example result.** A catalog recipe may ship `example/` with a small input and a saved result (`example/result.json` in the run-record shape plus any files). The Recipe page shows it through the ordinary viewers, labelled "example, made on 2026-11-02 with qwen3:8b". No model needed to look.
- **What it may touch, before anything runs.** The first Set up of a recipe that is not built-in shows its permission list as a consent step: every entry that is not `input`, `output`, or `${inputs.x}` has its own line and a tick, and the recipe will not run until the user ticks them. Granted entries are remembered per recipe in the store. This is the same consent the import (§4.5) needs, built once.

### 4.5 Import from a folder or zip

**Get a recipe…** in the Library's menu. The user picks a folder or a `.zip`; the app copies it into a quarantine folder, refuses symlinks and paths outside the bundle, runs `migrate` and `check`, shows the Recipe page with the consent step from §4.4 and the `minVersion` line, and only then copies it into the user's recipes folder. No URL field. Sharing in the other direction is **Export as zip** on a recipe you own, which never includes keeps, runs, pins, or anything under `assets/` the user marks private.

## 5. Phases

| Phase | What ships | Sessions | Verified by |
|---|---|---|---|
| **0. Fix first** | F1 to F10 from §2, with their tests. | 1–2 | `cargo test`, `pnpm test`, the review-in-kept-stage and edit-kept-step runs live; a screenshot of a result with a remote image rendered as text. |
| **1. Format layer** | §3.1 fixtures, `migrate.rs`, the registry with Notes and Fix, `minVersion` enforced and stamped, `x-`, versioned stages with provenance, `user_version`, planner permissions whitelisted, slot `requires` union. | 2 | Every fixture loads and checks; a pre-stages recipe gets a Note and a Fix on screen; a planner answer with `write: ["~"]` is refused with a problem in words. |
| **2. The shelf and the archive** | The `shelf` table, one example recipe at install, Archive and Restore, the card menu, delete removes keeps, Welcome ends on the example. Job ids on run events (the watch-mid-run fix). | 1 | Fresh app data dir shows one recipe; archive, restore, delete on screen; screenshots. |
| **3. The catalog** | The `browse` place, filters, cards with needs and may-touch, Look (read-only Recipe page), Add to shelf, `community/` root, `CONTRIBUTING.md`, CODEOWNERS, the ruleset (on your yes). | 2 | Six built-ins and one sample browsable; a community recipe added through a PR and seen in the app; screenshots. |
| **4. Understanding and import** | The consent step, example results on two catalog recipes, Get a recipe (folder or zip) with quarantine, Export as zip, symlink refusal, `http.fetch` loopback rule, Codex wording. | 2 | An imported recipe with `read: ["~"]` cannot run until ticked; a zip with a symlinked prompt is refused; screenshots. |
| **5. Later** | A user-triggered "check the catalog for new recipes" against the repository's `main` (sha256 per bundle in a signed index), nested recipes in the catalog. | — | Not in this round. |

Phases 0 and 1 come before the `stages` merge is cut as 0.3.0, or 0.3.0 ships with F1 to F10 and the format layer lands in 0.4.0; that is a release call for Jeremy.

## 6. Decisions to confirm

- **One example on a fresh shelf, the rest in the catalog.** The six built-ins stop appearing by default. Proposed.
- **Catalog recipes sit on the shelf by reference**, never copied until Duplicate. Proposed; it is what makes "a newer built-in exists" cheap and keeps the shadowing problem from growing.
- **Archive hides, Delete removes, and only recipes you own can be deleted.** Proposed.
- **Community recipes arrive by pull request into the same public repository and ship with the app.** No fetch at run time in this round. Proposed; it follows the no-ecosystem-importer decision.
- **Notes with a Fix are a checker tier; `Draft::write` refuses only Problems.** Proposed; this is the deprecation machine.
- **`x-` is where a new object lives until it is settled.** Proposed.
- **A protected `main` with a ruleset.** Yours to say yes to.
- **The example recipe's name.** *Describe a document* proposed; "Example recipe" as a tag on its card, not in its name.

## 7. The ruleset, when you say yes

```sh
gh api -X POST repos/jeremym/stillroom/rulesets --input - <<'JSON'
{ "name": "main", "target": "branch", "enforcement": "active",
  "conditions": { "ref_name": { "include": ["~DEFAULT_BRANCH"], "exclude": [] } },
  "bypass_actors": [{ "actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "always" }],
  "rules": [ { "type": "deletion" }, { "type": "non_fast_forward" },
             { "type": "pull_request", "parameters": { "required_approving_review_count": 0,
               "dismiss_stale_reviews_on_push": false, "require_code_owner_review": true,
               "require_last_push_approval": false, "required_review_thread_resolution": false } } ] }
JSON
```

Actor id 5 is the repository admin role, so your own release pushes bypass the pull-request rule.

## 8. Risks

- **The catalog hides the showcase.** A new user who never opens Browse never meets the Learning Kit. Mitigation: the empty-ish shelf shows the Browse button in the primary style, and Welcome's last screen offers "or browse the catalog".
- **Two rows of provenance (recipe and stage) add fields to a format we are trying to calm.** Both are `source` objects of three keys and both start under `x-` until phase 3 proves them.
- **Fixtures rot.** The compat test is the one that must stay red when a change is breaking; a fixture is only replaced when a migration is written for it.
- **The ruleset blocks you.** Bypass is granted to the admin role, and the rule can be deleted with one call.
