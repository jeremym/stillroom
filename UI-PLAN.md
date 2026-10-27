# Stillroom UI redesign: the phosphor plan

Written 2026-10-27. Companion to `PLAN.md` (the product design). This document is the plan for the **surface**: what the user sees, where they click, and how the app explains itself. The engine, the recipe format, and the Rust checker do not change.

Status and decisions live in the vault at `Stillroom/projects/ui-redesign.md`.

**Approved by Jeremy on 2026-10-27** ("I like everything that you've done here. Let's go for it!"), with one addition: sections need to read as separate zones, so each zone sits on its own ground and consecutive zones alternate between two near-identical greens, and long lists and tables alternate row tint. **Phase 1 shipped the same day**; see section 8.

---

## 0. In one screen

**Problem.** The app is a single scrolling page of eleven panels. A new user sees a graph, a results banner, a trace table, a form, watches, history, usage, and a models table all at once, with nothing saying *what to do first*. Setting up a run, watching it, reading the results, and editing the recipe are the same screen in different moods.

**Direction.** One engine, **five places** the user can be, and a **Dial** at the top that both moves between them and shows where a run is in its life. The look is a green phosphor terminal (TRS-80, VT220, Fallout Pip-Boy) done with care for legibility: crisp text with a soft halo, very few things on screen, and every clickable thing unmistakable.

**Five phases**, each shippable and screenshot-verified:

| Phase | Name | What lands |
|---|---|---|
| 1 | **Glass** | The design kit (tokens, type, glow, 14 components, 24 glyphs, logo) and the new shell with the Dial. No feature changes. |
| 2 | **Power on** | First-run wizard and the Set up place: pick a recipe, fill inputs, see models and permissions, one big RUN. |
| 3 | **Running and Results** | Live run as a lit flow plus a teletype log; results as a printout stack; the review checkpoint as an amber "your call" state. |
| 4 | **Open the door** | Flow View redrawn: in and out lanes, a glyph per step kind, local versus cloud marks on every AI step, inspector and edit mode restyled. |
| 5 | **Library and polish** | Recipes, watches, history, models, settings in one place; empty states that teach; the kit extracted for other apps. |

**Non-negotiables carried from the product principles.** Status is always a word, never a color alone. Friendly term first, technical term in parentheses. Anything that leaves the machine is flagged. Validity stays on the Rust side.

---

## 1. Why the current UI is hard to learn

Observed from `src/App.tsx` and the components, not from guesswork:

- **No places, only panels.** Flow, result banner, review, artifacts, trace, recipe form, watches, errors, history, usage, models, footer, in one column. The eye has no anchor.
- **The first thing on screen is the hardest thing.** Flow View sits at the top, above the form that starts a run. A beginner meets the execution graph before they meet the Run button.
- **Run has no stage.** Starting, running, and finished differ only by a sentence of muted text and a table filling in.
- **Results are a tab strip under a sentence.** The thing the user came for ("notes, slides, audio") has less weight than the trace.
- **Step kinds are words in small caps.** "Tool", "AI step", "Agent" are correct but nothing makes "a program on disk" and "a model thinking" *look* different. Local versus cloud is not visible at all (there is no cloud yet, but the design must have the slot).
- **Inputs and results are node shapes.** Dashed box in, pill out. Not legible as "this is where things come in" and "this is what you get".
- **Models panel is a developer table.** Size, context, capabilities. It should answer "is the thinking machine ready?"
- **Light paper theme at 16px.** Fine for a document, wrong for a workbench, and it fights the brand.
- **Window opens at 800×600.** The flow panel is designed for 1280.

---

## 2. The look: phosphor, done legibly

### 2.1 Mood

A dark glass screen. Text and lines are phosphor green with a soft halo, as if lit from behind. Chrome is sparse: thin bezels, bracketed fields, block cursors, status words in small caps. Motion is slow and deliberate: a dial that turns, a line that pulses while a step runs, a log that types. Nothing flickers.

Content the recipe produced (notes, a slide, a table) renders as **paper inside the glass**: a lighter panel with dark text, so what the user exports looks like what they saw. The machine is green; the output is paper.

### 2.2 Legibility rules (Jeremy's eyesight is the first test user)

- **Crisp core, soft halo.** Glow is two layered shadows: a zero-blur core shadow for weight, a wide low-alpha halo for light. Never a blur on the glyph itself. Body text gets the smallest halo or none.
- **Contrast at or above 7:1** for body text (phosphor `#5cff7a` on `#060a07` is about 14:1). Muted text no lower than 4.5:1 (`#4e9a60` on the same ground is about 5:1).
- **Body 17px minimum, line height 1.55**, labels 15px, status words 13px small caps with wide tracking. A **Larger text** switch in settings scales the whole kit by 1.15 and 1.3 (rem based, one token).
- **Scanlines off by default.** A 3 percent overlay is available as a toggle and never covers reading panels.
- **No flicker, ever.** `prefers-reduced-motion` turns the dial spin into a crossfade, the teletype into an instant print, the boot into a single frame.
- **Three themes, same components:** **Green** (P1 phosphor, default), **Amber** (P3), and **Paper** (the present light look, kept as a high-contrast fallback). Theme is one attribute on `:root`.
- **Color is never the only signal.** Every status has a word and most have a glyph. Amber means "you" (review, needs attention) and "leaves this machine" (cloud). Red is failure only.

### 2.3 Tokens (the whole palette)

```
--ground       #060a07   screen glass
--ground-2     #0b120d   raised panel
--bezel        #1a2b1e   borders, rules
--phos         #5cff7a   primary text, lines, glyphs
--phos-dim     #4e9a60   secondary text
--phos-faint   #244a2e   disabled, grid, scanline
--phos-fill    #0f2a16   selected row, button hover
--halo         rgba(92,255,122,.35)
--amber        #ffb347   needs you, cloud, caution
--amber-halo   rgba(255,179,71,.35)
--red          #ff5c5c   failure only
--paper        #f4f1e6   artifact ground
--ink          #1c1c1e   artifact text
```

Type: **display** in a terminal face for the logo, the Dial, and status words (VT323 or Glass TTY VT220, both free; bundled, never fetched); **body and code** in one legible monospace with real italics and clear 0/O, 1/l (JetBrains Mono or IBM Plex Mono, OFL; bundled). Everything is mono: it is a terminal.

Glow recipe, used by text, borders, and SVG strokes:

```css
.glow      { text-shadow: 0 0 0 currentColor, 0 0 10px var(--halo); }
.glow-box  { box-shadow: 0 0 0 1px var(--phos), 0 0 14px var(--halo); }
.glow-svg  { filter: drop-shadow(0 0 5px var(--halo)); }   /* hover and active only */
```

Performance rule: `drop-shadow` on SVG and `backdrop-filter` only on things that are few and interactive. Static glow is `text-shadow` and `box-shadow`, which WKWebView handles well. Test on the Inbox Triage graph (the largest) before accepting any effect.

### 2.4 Logo and glyphs

- **Logomark:** an alembic (a still) drawn as a two-stroke outline, the drop at the spout the only filled shape. In phosphor it reads as a lit tube. Wordmark `STILLROOM` in the display face with wide tracking. App icon: the alembic on glass with a halo, exported from the same SVG at every size.
- **Glyph set (24, one SVG sprite, 2px stroke, `currentColor`):** file, folder, text, links, program (floppy), ai (waveform brain), agent (waveform with a wrench), for-each (stacked frames), approval (hand), result (printout page), audio, slides, diagram, table, notes, local (house), cloud (cloud with an up-arrow), llama (provider mark for Ollama), watch (eye), history (clock), models (chip), settings (dial), warning, run (power symbol).
- **Ollama mark.** Draw our own simple llama outline for the "local, via Ollama" badge rather than lifting Ollama's logo; check their brand guidelines before using the real one anywhere.

### 2.5 The Dial (top navigation)

The thing the user asked for: "not just tabs". A horizontal **drum** of five place names behind a fixed **lens** in the center. The drum rotates (CSS 3D, `rotateX` on a cylinder of labels; crossfade under reduced motion); the place under the lens is where you are. The lens glows green, turns amber when a run needs you, and shows a thin progress arc while a run is in flight.

```
            ╭─────────── LENS ───────────╮
 LIBRARY   SET UP  │   RUNNING   │  RESULTS   OPEN THE DOOR
            ╰───────────────────────────╯
```

- Click a name or press ←/→ to turn it. It is a `role="tablist"`; the drum is decoration over real tabs.
- **It moves by itself** at the right moments: RUN turns it to Running; a finished run turns it to Results; a review turns it to Results with the lens amber. The user learns the lifecycle by watching the dial follow it.
- Places that have nothing yet are dim with a one-line reason ("No run yet. Set up a recipe and press RUN."). Nothing is hidden, nothing is a dead click.
- Below the lens, one line of **breadcrumb context** in the display face: `SUMMARIZE A FILE · notes.md · 3 steps · 1 model, local`.

### 2.6 Interaction rules

- **Few targets, all obvious.** A screen has one primary action (filled phosphor, dark text, glowing) and at most three secondary (outlined). Links are underlined and glow on hover.
- **Fields jump out.** Inputs are bracketed `[ ... ]` with a block cursor, a dim prompt label above, and a full glow on focus. File pickers are a wide button with the glyph of what they take and the chosen path printed under it in mono.
- **Toggles are words:** `[ ON ]` / `[ OFF ]`, the active state filled.
- **Selects** keep the native control for accessibility, styled to the bezel; where the choice is small and meaningful (theme, model), a segmented control with glyphs replaces it.
- **Progress is block characters** `▓▓▓▒░░` in the display face, with a percent or a count beside it.
- **Confirmations are inline** ("Really remove" replaces the button), never a modal dialog.
- **Errors print in red in the log** and in a red-rimmed line near the control that caused them, with the plain-language reason first and the technical text in a fold.

---

## 3. The five places

### 3.1 Library (the shelf)

Everything that persists, one scroll with four headed sections:

- **Recipes** as cards: glyph, name, one line of what it gives you ("notes, flashcards, slides, narration from one document"), the step count, and badges for what it needs (a file, a folder, links) and whether every AI step can be filled by an installed model. Built-in versus yours is a small tag. Click a card: the Dial turns to Set up with that recipe loaded.
- **Watches** (recipes that run on their own when something lands in a folder) with the word status, last run, Pause, Run now, Remove.
- **History**: the last twenty runs, each a line with the recipe glyph, when, the status word, and the number of results. Click: the Dial turns to Results.
- **Models and settings**: a readiness panel, not a table. "Ollama: running, 4 models" with a green llama; each model one line with the capability glyphs it has (chat, structured output, tools, thinking) and its size in plain words; **Cloud providers: none configured (off)** in amber-tagged text so the slot is visible before it exists. Theme, text size, scanlines, "run the first-time setup again".

### 3.2 Set up

The one form, built from the recipe's declarations as today, but staged:

1. **Recipe** header: glyph, name, the plain-language promise, and "what it needs" as a checklist that ticks as inputs are filled.
2. **Inputs**: bracketed fields and pickers. Required first, optional folded.
3. **Models for this run**: one row per slot. Each row shows the slot's job in plain words, the resolved model with its **LOCAL** (house plus llama) or **CLOUD** (amber cloud) mark, and a change button. "Auto" explains itself: "Auto: picks the recommended model if installed, else the best installed for accuracy".
4. **What it may touch**: permissions in one sentence as today, with glyphs for folders and hosts, and the line "Nothing else is touched."
5. **What will happen**: the steps as a horizontal strip of glyphs with names, local or cloud marked, and the estimate ("about 3 minutes, based on 4 earlier runs").
6. **RUN** (big, glowing) and, if the recipe has a folder input, **WATCH THIS FOLDER** beside it. "Reuse unchanged steps" stays as a `[ ON ]` toggle with its parenthetical.

Disabled RUN always says why, in the button: `RUN · needs a file`, `RUN · no installed model can write`.

### 3.3 Running

A stage, not a table:

- **Top:** the flow at reading size, steps lighting up as they run (the glyph pulses, the edge into it carries moving dashes, done steps settle to steady). A map shows `3 of 7` inside its frame.
- **Bottom:** the **teletype** (the execution trace, live): one line per event, typed in as it arrives: `14:02:11  ▶ Extract the spine · AI step · qwen3:8b · local`, `14:03:20  ✓ Extract the spine · 69 s · 1.2k → 480 tokens`, `14:03:20  ↻ Write notes · reused from run 8f2c`. Agent tool calls indent beneath their step. Failure prints red with the reason.
- **Lens:** a progress arc and `STEP 3 OF 7`. The window title carries the same, so a minimized app still reports.
- **Stop** is the one control (outlined, red on hover, inline confirm).
- A run started by a watch shows `STARTED BY THE FOLDER WATCH after 2 files landed` in the breadcrumb.

### 3.4 Results

- **Header:** `LATEST RESULT · Learning Kit · today 14:06 · 7 steps, 4 reused` and, when relevant, `includes your edit to Script` or `started by the folder watch`.
- **If the run needs you:** the whole place takes the amber rim. The review (your checkpoint) is the first thing: the title, the plain sentence ("Nothing is written until you approve"), the items as cards with tick boxes, `APPROVE 3 OF 4 AND CONTINUE` glowing amber. New items carry a `NEW` tag; earlier decisions say so.
- **Printout stack:** each result is a tab on a paper panel. Tabs carry the output glyph (notes, slides, diagram, audio, table) and the word `ready`, `pending`, or `edited by you`. The viewers are the existing ones restyled: notes and tables on paper, slides as the white slide with a green list of titles beside it, diagrams themed green on glass (Mermaid theme variables) with an export that renders on paper, audio as a green waveform bar with the script below.
- **Edit** stays in place; "regenerates 3 downstream steps: Slides, Script, Audio" prints before the button, and saving turns the Dial to Running.
- **How this was made** is one quiet link under the stack: it turns the Dial to Open the door with this run painted on.

### 3.5 Open the door (Flow View, Understand and Build)

- **Lanes.** A left **IN** lane and a right **OUT** lane, each with a dim vertical rule and a header glyph. Input nodes are sockets: a right-pointing arrow glyph plus the input's kind glyph. Result nodes are printouts: a page glyph with an outbound arrow. Nobody has to learn that dashed means input.
- **Step nodes by kind.** Same box, different glyph and subtitle rule:

  | Kind | Glyph | Label | Subtitle |
  |---|---|---|---|
  | tool | floppy | `PROGRAM` | the tool id |
  | model | waveform brain | `AI STEP · LOCAL` or `AI STEP · CLOUD` (amber) | model name, provider mark |
  | agent | waveform with wrench | `AI AGENT · LOCAL/CLOUD` | its tools |
  | map | stacked frames | `FOR EACH` | `3 of 7` while running |
  | review | hand | `YOUR APPROVAL` (amber) | what it shows you |

  A `PROGRAM` never pulses. An `AI STEP` pulses while thinking. A cloud step always carries the amber cloud and the words `leaves this Mac`.
- **Edges.** In-lane edges are dotted until data has passed, then solid. Step-to-step edges are solid. A running edge carries moving dashes toward the running step. Held and skipped edges are dim dashes with the status word on the node.
- **Inspector** (the details panel) in the same bezel: title, kind explained in one sentence (the existing `KIND_EXPLAINED` text), the model with its mark and a change control, inputs and what came out, with the prompt in a fold.
- **Edit mode** is the existing editor in the new kit: the bezel turns to a dashed amber "editing" rim, the problem list prints in the teletype style, handles glow, the Save dialog is an inline card. Nothing about the draft model changes.
- The canvas stays static and page-scrolling (a settled decision).

### 3.6 Power on (first run)

The Set up place with training wheels, not a second UI. Shown once, available again from settings.

0. **Boot:** the alembic glows up, `STILLROOM` types in, `Describe the goal. Get the result. Open the door.` One button: `BEGIN`. Under reduced motion it is one frame. Under 1.5 seconds either way.
1. **Where will the thinking happen?** Checks Ollama. Found: green llama, `4 models ready`. Not found: install link and `I've installed it, check again`. No models: recommend one small chat model and offer `PULL IT FOR ME` with a block-character progress bar (Ollama's HTTP pull endpoint streams progress; the adapter can call it). One line: `Cloud models are optional, off, and marked amber whenever one is used.`
2. **What should it make first?** The four recipe cards. Summarize a File is pre-selected as the fastest.
3. **Pick the file.** One bracketed picker.
4. **Here is what will happen.** The step strip, the permission sentence, the estimate, `RUN`.
5. The Dial turns to Running, then Results, with one pointer: `Open the door to see how this was made.`

---

## 4. The phases in detail

Each phase ends with: `cargo test --workspace`, `pnpm test`, `pnpm exec tsc --noEmit`, `pnpm build`, a live run against Ollama, a screenshot of every place touched at 1280×840, a test of the Green and Paper themes, a `prefers-reduced-motion` check, and a local commit. Sizes are rough sessions of focused work.

### Phase 1: Glass (the kit and the shell) · about 3 sessions

Goal: the app looks like the new thing and is organized into places, with every existing feature still reachable. No behavior changes.

- `src/kit/` with no Stillroom imports (so it can move to its own package later): `tokens.css`, `type.css`, `glow.css`, and components `Dial`, `Bezel` (panel), `Button`, `Field`, `Picker`, `Toggle`, `Segmented`, `Badge`, `StatusWord`, `Progress`, `Teletype`, `Table`, `Tabs`, `Card`, `Glyph` (sprite), `Logo`.
- Fonts bundled under `public/fonts/` with `@font-face`; no network fetch.
- Theme attribute and the three themes; a text-size token; reduced-motion handling in one place.
- `App.tsx` becomes a shell: Dial, breadcrumb line, one place rendered at a time, the error line, the footer. Existing panels are moved into the places unchanged in logic, restyled in the kit.
- Window defaults to 1280×840, minimum 1040×700, size remembered (`tauri-plugin-window-state`).
- Logo SVG and app icons regenerated from it.
- Done when: all five places exist and the old features work inside them; screenshots of each; no new TypeScript validator or engine change.

### Phase 2: Power on (wizard and Set up) · about 2 sessions

- Set up place rebuilt in stages as in 3.2; the recipe form logic (`RecipePanel`) reused, the layout new.
- Model rows with LOCAL/CLOUD marks; a `provider.kind` of local or cloud exposed by the Rust side (today everything is local; the field makes the cloud slot visible before it exists).
- Disabled RUN explains itself in the button.
- Wizard: a `firstRun` flag in the store; the boot screen; the Ollama check and in-app pull with progress through the adapter; the recipe cards; the final RUN. "Run first-time setup again" in settings.
- Done when: a new profile goes from launch to a Summarize a File result without reading anything outside the app; Jeremy tries it with someone who has never seen it.

### Phase 3: Running and Results · about 3 sessions

- Running place: flow at the top reusing the graph layer, teletype at the bottom fed by the `run:step` events and the stored records, lens progress arc, window title progress, Stop.
- Results place: header line, amber review state, printout stack with the viewers on paper, diagram theme for Mermaid on glass, edit flow turning the dial.
- The Dial moves itself on RUN, finish, and needs-review.
- Lazy-load Mermaid and pptxgenjs while in here (a known wart).
- Done when: Learning Kit and Inbox Triage both run end to end through Running into Results with the review approved from the amber state; screenshots at each stage.

### Phase 4: Open the door · about 3 sessions

- Lanes, glyphs per kind, local and cloud marks, edge styles and the running dash animation, status words preserved.
- Inspector and edit mode restyled; problems printed teletype-style; Save as inline card.
- Node layout constants revisited for the new type size; the "Draft a reply" crowding wart fixed.
- Done when: all four built-in recipes are readable at a glance (someone can say which steps are AI and which are programs without reading the labels); edit, wire, save, and load back still work; the Inbox Triage graph scrolls smoothly with glow on.

### Phase 5: Library and polish · about 2 sessions

- Library place: recipe cards, watches, history, models readiness, settings.
- Empty states that teach, one per place.
- Keyboard: ←/→ turns the Dial, `R` runs when Set up is complete, `Esc` closes the inspector.
- Sweep: every control has a glyph or a word, every status is a word, every jargon term has its parenthetical.
- Extract `src/kit/` into its own repository with a README and the style tile, so the next app starts from it.
- Done when: a fresh screenshot set replaces the ones in the vault and `README.md`; the kit builds alone.

---

## 5. Decisions (confirmed 2026-10-27)

1. **Phosphor green on glass is the brand**, with amber and paper themes built in. Legibility rules in 2.2 are part of the decision.
2. **Five places, one Dial.** Library, Set up, Running, Results, Open the door. The Dial follows the run by itself.
3. **The machine is green, the output is paper.** Artifact content renders light-on-dark never; it renders as paper inside the glass.
4. **Amber means you or away.** Review and attention, and anything that leaves this Mac. Red is failure only.
5. **Step kinds have glyphs**: program, AI step, AI agent, for each, your approval, and every AI step carries a LOCAL or CLOUD mark.
6. **The kit is a product of its own**, built in `src/kit/` with no app imports and extracted in phase 5 for the other apps.
7. **Fonts bundled, never fetched.** Local-first applies to the UI.
8. **Zones alternate.** Each section of a place is a zone on its own ground; consecutive zones alternate `--ground-2` and `--ground-3`, and rows in lists and tables alternate a 4 percent tint. Jeremy's addition on approval.

Working name for the kit: **P1** (the green phosphor designation). "Phosphor" alone collides with the Phosphor Icons library.

---

## 6. Risks and how the plan handles them

| Risk | Handling |
|---|---|
| Glow hurts legibility for low vision | Crisp core plus halo, body text with little or no halo, 17px floor, text-size switch, Paper theme as a fallback. Jeremy is the first test. |
| Retro look reads as a toy | Restraint: one accent, no scanlines by default, no flicker, slow motion, real information density in the teletype and inspector. |
| WKWebView performance with many shadows | Static glow via text and box shadows only; `drop-shadow` on hover; measured on the Inbox Triage graph before acceptance. |
| The Dial becomes a gimmick that hides navigation | It is a real tablist with visible labels for all five places at all times; the drum is decoration; keyboard and click both work. |
| Scope creep into features | Phase gates list only UI work plus the two small Rust additions (provider kind, pull progress). The planner is still the next engine milestone after this. |
| Second validator in TypeScript | None added. Problems still come from `check_recipe_draft`. |

---

## 7. First steps (phase 1, session 1)

1. `src/kit/tokens.css`, `type.css`, `glow.css` and the three themes; bundle the two fonts.
2. `Glyph` sprite with the 24 glyphs and the `Logo`.
3. `Dial` with keyboard support, reduced-motion crossfade, and the lens states (green, amber, progress arc).
4. Reshape `App.tsx` into the shell with five places and move the existing panels in.
5. Window size and state plugin.
6. Screenshots of all five places; commit.

The style tile at `docs/mockups/p1-style-tile.html` shows the tokens, the Dial, a step node of each kind, a field, a button, and the teletype, so the direction can be judged before any code moves.

---

## 8. Status

### Phase 1: Glass · shipped 2026-10-27

- `src/kit/`: `tokens.css` (three themes, text-size scale), `glow.css`, `kit.css` (zones, fields, buttons, tags, tables, tabs, paper, cards, progress, teletype, the Dial, scrollbars), `Glyph` (33 glyphs with `inputGlyph`, `stepGlyph`, `outputGlyph`), `Logo`, `Dial`, `Zone`/`Empty`, `theme.ts`. No Stillroom imports.
- Fonts bundled through `@fontsource/vt323` and `@fontsource/jetbrains-mono`; nothing fetched at run time.
- `App.tsx` is the shell: header with the provider status, the Dial, one place at a time, footer. The Dial turns by itself on RUN, on finish, and when a run needs you; the lens goes amber for a review and shows a progress line while running.
- Library has recipe cards (with input glyphs and a "needs a model" tag), watches, history, usage, models, and a Look zone (theme, text size, scanlines). Set up is the recipe form with a RUN button that says why it is disabled. Running shows the lit flow and the trace. Results shows the review, the printout stack on paper, and two links to how it was made. Open the door is Flow View with edit mode.
- Flow nodes carry a glyph per kind and per input or result type, running edges carry moving dashes, a running step's glyph pulses. The Dial chose fixed labels with a sliding lens over a scrolling drum, so all five places stay visible and clickable.
- Window opens at 1280×840, minimum 1040×700, position and size remembered (`tauri-plugin-window-state`). New app icon from the alembic mark.
- Verified: `cargo test --workspace` (8 suites), `pnpm test` (16), `tsc`, `pnpm build`, a live Summarize a File run through the UI, screenshots of all five places in `docs/screenshots/phase1-*.png`.
- Deferred to later phases: Stop (needs a cancel command), teletype log formatting (phase 3), lanes and LOCAL/CLOUD marks (phase 4), Library readiness panel (phase 5).
