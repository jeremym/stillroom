# Contributing

Stillroom is developed in the open. Bug reports and pull requests are welcome. The default branch is protected: changes arrive by pull request, and anything under `recipes/`, `community/`, `stages/`, `schemas/`, or `prompts/` is reviewed by the maintainer before it ships.

## Community recipes

A community recipe is a recipe folder under `community/<id>/` that ships inside the app and appears in its catalog under the **community** row with your name on it. There is no registry and no fetching at run time: a recipe reaches users only through this repository, read by a person first. That is deliberate (see `PLAN.md` §8 and §11).

To offer one:

1. Make it in Stillroom, run it until you are happy with it, then **Reveal in Finder** from its card and copy the folder into `community/<id>/`. The id is the folder name: lowercase, hyphens, at most 64 characters.
2. Open a pull request titled `community: <recipe name>`. Say in a few sentences what the recipe is for and what you ran it on.

### What the review checks

The maintainer runs the checklist below. A recipe that fails one is sent back with a note, not refused outright.

- `recipe.yaml` opens in the app and its Recipe card shows **no problems and no notes** (the checker: `Draft::check`).
- `author.name` is filled in, and `license` is set (MIT unless you have a reason).
- **Permissions are the smallest that work.** `filesystem.read` and `write` use `input` and `output`, or `${inputs.<name>}`; a literal path needs a sentence in the pull request saying why. `network` lists only the hosts the recipe needs, or `input` for the user's own links. No `shell`, no `cloudModels`, no `credentials`.
- **Prompts are read for injection.** A prompt instructs the model about the task; it never tells the model to ignore its instructions, read files the recipe did not ask for, or send anything anywhere.
- Every `stages[].why` reads as one plain sentence a first-time user understands.
- `runtime.minVersion` is set (the app stamps it on save).
- Model slots say what they need (`requires`) and do not name a provider; a `recommended` model is fine.
- Nothing private travels with the recipe: no run keeps, no results, no personal files under `assets/`.
- The maintainer runs it once live against local models before merging.

### Updating a community recipe

Open a pull request to the same folder and bump `version`. Users who added the earlier version to their shelf are told a newer one exists; their edited copies are never overwritten.

## Code

Before a pull request: `cargo test --workspace`, `pnpm test`, `pnpm exec tsc --noEmit`, `pnpm build`. Use pnpm, never npm or yarn. Commit messages say what changed in plain words; no generated-by lines.
