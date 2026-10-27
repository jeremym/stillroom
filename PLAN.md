# Stillroom

**A goal-first, local-first AI workflow studio for people who have never built a workflow.**

> Describe the goal. Get the result. Open the door.

Plan revised 2026-10-26. Claims about standards, libraries, and competitors below were fact-checked against primary sources on that date. Items marked **(unverified)** came only from secondary sources.

---

## 0. What this is, in one paragraph

Stillroom is a desktop application (Mac first, Apple Silicon) that does for agentic AI what n8n, Dify, and Langflow do for developers, but starts from the other end. You describe what you want to accomplish. The app plans the steps, picks the models and tools, runs the workflow, and hands you real outputs: documents, reports, slides, diagrams, audio, data files, automations. Then, only if you want, it lets you open the door and see the machine that made them: the graph, the prompts, the model assignments, the data flowing between steps. Everything it builds is saved as a portable **Recipe** you can edit, rerun, and share.

Stillroom is **not** a learning app. A "learning kit" recipe (turn source material into notes, slides, diagrams, flashcards, and a narrated lesson) is the first showcase because it exercises nearly every hard problem at once. The architecture is general from day one.

A *stillroom* was the room in a large house where the household kept its recipe book and actually made things from it: distillations, preserves, remedies. Recipes were written to be handed on. That is the product.

---

## 1. The differentiator: hide the plumbing, not the intelligence

The visual workflow builders that already exist are capable. The opportunity is not another node editor. It is to make the editor optional.

| Existing product | What it offers (Oct 2026) | Opportunity for Stillroom |
|---|---|---|
| n8n (~2.43) | Open-core automation, AI Agent and Guardrails nodes, evaluations, standalone agents in preview since Sep 2026 | Goal-first, local desktop, no server to host |
| Dify (1.17) | AI apps, agent workflows, RAG, Human Input node for pause-and-review | No Docker, consumer UX, offline by default |
| Langflow (1.12, IBM) | Visual agent graphs, custom Python components | Non-developer interaction |
| ComfyUI / Comfy Desktop | Graph-based media pipelines; **App Mode / App Builder / ComfyHub** (Mar 2026) hides the graph and ships a workflow as an app; 4M users | One orchestration model across text, data, and media, not only images |
| Claude Cowork + plugins | Local desktop, goal-first, hidden plumbing, Agent Skills, local folder access; GA Apr 2026 | Model-independent and local-inference-first; shareable, inspectable recipes rather than a closed assistant |
| Jan / AnythingLLM / Open WebUI | Local chat with bundled llama.cpp, RAG, basic agents | Goal-first workflow authoring, which none of them have |
| Hermes Agent / OpenClaw | Local personal assistants, chat-first, huge community skill registries; Hermes writes its own skills after hard tasks | Reproducible, inspectable workflows with real artifacts and provenance; permission-gated recipes instead of an open registry (ClawHub suffered a 341-skill supply-chain attack) |

Dead or dying, do not benchmark against: Flowise (EOL Aug 31, 2026), Relay.app (shut down Sep 2026), OpenAI Agent Builder (deprecated June 2026, unavailable Nov 30, 2026), Google Opal (still a cloud-only Labs experiment).

**The four closest positioning threats** are Claude Cowork, ComfyUI App Mode, n8n standalone agents, and the installed base of local-first chat apps. Stillroom's wedge against all four is the same: a goal in, real artifacts out, a portable and inspectable recipe underneath, and local models by default.

### Progressive disclosure

Three levels of control over one underlying workflow, not three products:

1. **Do.** Describe a goal, receive useful outputs.
2. **Understand.** See the decisions, tools, and data behind each output.
3. **Build.** Modify steps, models, prompts, connections, and rules.

A beginner never needs to see a graph. A technical user can inspect nearly everything. And a recipe never depends on one model, one vendor, or one computer.

---

## 2. The user experience: a creative workbench

The interface should feel like a music production app or a creative studio, not an automation dashboard.

**Navigation:** Home, Projects, Recipes, Tools, Models.

**A project screen** shows the goal in plain language, a single primary action ("Build it"), the generated artifacts as cards, and one quiet link: "View how this was made · 7 steps · 3 models."

### The three primary views

- **Workspace.** The everyday experience. Sources, outputs, and editable artifacts. Users edit individual outputs without touching orchestration.
- **Flow View.** Zoom into the system. Steps, tools, branches, dependencies, model assignments, and live execution traces as connected nodes.
- **Workbench.** The expert view. Prompt versions, debugging, input/output inspection, model comparisons, evaluations, performance.

These are different interfaces over the same engine.

---

## 3. Recipes: the unit of everything

A **Recipe** is a complete, reusable process for accomplishing a goal. It declares inputs, steps, the capabilities each step needs, dependencies, and the artifacts it produces.

Example recipes beyond the showcase, all on the same engine:

- Weekly business report from a folder of CSVs and a Notion export.
- Research brief from a set of URLs, PDFs, and a podcast episode.
- Blog post plus social snippets plus a narrated audio version from an outline.
- Meeting recording to summary, action items, and calendar entries.
- Photo folder to captioned, tagged, organized library.
- Personal automation: watch a folder, transform what lands there, file it.

### Showcase recipe: Learning Kit

Sources (documents, URLs, audio, video, notes) go to a planning step that extracts facts, identifies concepts, plans explanations, and selects tools. Parallel steps then summarize, visualize, narrate, build slides, build flashcards, and write a quiz. A review step checks accuracy, source references, and missing concepts. Output is an editable kit.

### Knowledge Spine: a shared structured representation

Six independent steps should not each read the original PDF and produce unrelated summaries. Instead, the source is converted once into concepts, claims, references, and relationships. Every output modality is a view of the same knowledge.

That makes this possible:

> "Our explanation of refresh tokens is wrong. Fix it, then update my slides, diagram, flashcards, and narrated lesson."

The app knows which artifacts depend on which concept and regenerates only those.

Two graphs, kept separate: the **knowledge graph** (relationships between ideas) and the **execution graph** (the workflow and tools that produce outputs). They interact but are different things.

For learning specifically, the app can track which concepts a user understands and schedule review with **FSRS-6**, the current production spaced-repetition algorithm used by Anki 26.09 and RemNote (FSRS-7 exists as a draft but is unreleased). Rust crate `fsrs` 6.6.2; TypeScript `ts-fsrs` 5.4.1 stable, 6.0 in beta.

---

## 4. Standards to build on

There is still **no vendor-neutral standard for portable agent workflow definitions** as of October 2026. Oracle's Open Agent Specification is the nearest candidate (declarative JSON/YAML for agents and flows, adapters for LangGraph/AutoGen/CrewAI). Microsoft's Agent Framework has declarative workflow YAML, but it is runtime-specific. OpenAI's Agent Builder exports code, not a spec, and is being shut down. MCP, A2A, and Agent Skills standardize tools, agent-to-agent messaging, and skill packaging respectively, not workflows. That gap is the layer Stillroom defines. Also checked and ruled out: Haystack's pipeline YAML is Haystack-specific (its docs warn to load only from trusted sources), and mozilla.ai's any-agent wraps frameworks without defining a format.

| Standard | What it gives us | Status (Oct 2026) | Priority |
|---|---|---|---|
| **MCP** | Connect to tools, resources, prompts | Spec revision 2026-07-28. Stateless core (no handshake or session IDs, per-request `_meta`, Multi Round-Trip Requests replace server-initiated calls), formal extensions framework (Tasks, Apps, Auth, Skills now ship as extensions), header-based routing, cacheable list results. Roots, Sampling, Logging, HTTP+SSE transport deprecated. Governed by the Agentic AI Foundation (Linux Foundation) since Dec 2025. | First |
| **Agent Skills** | Portable instructions in `SKILL.md` folders | Open standard since Dec 2025; donated to the Agentic AI Foundation Oct 22, 2026. 70+ platforms integrate it. Spec: agentskills.io/specification | First |
| **JSON Schema** | Typed inputs and outputs, independent of vendor | Stable | First |
| **OpenTelemetry GenAI conventions** | Standard traces for model calls, agents, tools, workflows | `gen_ai.*` spans exist (`invoke_agent`, `invoke_workflow`, `execute_tool`, `load_skill`) but are **Development status, no stable release**; moved to the separate `semantic-conventions-genai` repo in June 2026. Attribute names may change. | Early, behind an adapter |
| **OpenAPI** | Ordinary HTTP services as tools | Stable | Early |
| **A2A** | Delegate to independent external agents | v1.0.0 Mar 12, 2026; v1.0.1 May 28, 2026 is current. Under the Agentic AI Foundation since Aug 2026. | Later |
| **Open Agent Spec (Oracle)** | Possible import/export target for flows | Early, Apache-2.0 | Watch; consider as export target |

### How the layers fit

- **MCP** = What can I use?
- **Skills** = How do I do it?
- **A2A** = Who else can help?
- **Recipe format** = How does everything fit together?

Stillroom does not replace MCP or reinvent agents. It provides a friendly way to assemble their capabilities into useful, portable systems.

### Agent Skills, concretely

A skill is a directory `skill-name/SKILL.md` plus optional `scripts/`, `references/`, `assets/`. Frontmatter: `name` (required, lowercase, must match the directory), `description` (required), optional `license`, `compatibility`, `metadata`, `allowed-tools`. Loading is progressive: name and description at startup, full body on activation, bundled files only when referenced. A loader is roughly 100 lines of Rust with serde_yaml; the third-party `agent-skills` crate (0.2) can validate. MCP now has an official "Skills over MCP" extension for discovering skills from servers, which Stillroom should support.

---

## 5. A model-independent orchestration engine

The most important engineering component. A recipe step declares the **capability** it needs. Which model fills it is decided deterministically before the run, in a fixed order, and never by the planning model. The planner proposes steps. It does not choose providers.

| Capability | User-facing description | Local provider | Cloud fallback |
|---|---|---|---|
| Reasoning | Analyze complex information | Ollama (MLX on Apple Silicon), LM Studio, llama-server | Any OpenAI- or Anthropic-compatible API |
| Writing | Create concise explanations | Same | Same |
| Vision | Understand an image | Local VLM via Ollama | Cloud vision model |
| Speech | Read a script aloud | Kokoro (default), Chatterbox (voice cloning), system `AVSpeechSynthesizer` | Cloud TTS |
| Transcription | Convert speech to text | whisper-rs (Core ML + Metal), Apple SpeechAnalyzer on macOS 26+, Parakeet | Cloud STT |
| Illustration | Generate visual assets | Local diffusion | Cloud image model |
| Embedding | Find related source material | Local embedding model via Ollama | Cloud embeddings |

### Local runtimes: one adapter covers most of them

Ollama (`localhost:11434/v1`), LM Studio (`localhost:1234/v1`), llama.cpp `llama-server`, `mlx_lm.server`, and vllm-mlx all expose OpenAI-compatible chat, completions, and embeddings endpoints. One adapter with a configurable base URL handles all of them. Ollama is the default because it also exposes a native API with schema-constrained structured outputs (`format` accepts a full JSON Schema), tool calling, thinking levels, and since v0.40 (Sep 2026) runs MLX by default on Apple Silicon for supported model families, with llama.cpp as the fallback path.

**Apple Foundation Models** (macOS 26, ~3B on-device) is Swift-only with no HTTP API. Using it requires a small Swift sidecar or FFI bridge, not the OpenAI adapter. Worth it later for zero-download, zero-config first-run.

`mlx_lm.server` is explicitly "not for production"; do not make it a default backend.

### Model slots and the resolution order

Each step is assigned an abstract slot. A slot is the portability mechanism: it is what lets a recipe run on someone else's machine. It is not a license for the AI to pick models.

```yaml
modelSlots:
  planner:
    requires: [text, structured_output, tool_calling]
    preference: accuracy
    recommended: qwen3:32b        # author's suggestion, shown as a default
  quick_writer:
    requires: [text]
    preference: speed
  narrator:
    requires: [text_to_speech]
    preference: local
  visualizer:
    requires: [image_generation]
    preference: quality
```

Resolution happens before the run, in this order. The first match wins.

1. **Pinned.** The user chose this model for this step in this project. Always wins. Pins are stored with the project, not the recipe, so sharing a recipe never leaks your choices.
2. **Recipe recommendation.** The author's `recommended` model, if installed. Shown as a default the user can accept or change.
3. **Capability resolve.** Only when nothing is pinned and the recommendation is not installed: pick from installed providers that satisfy `requires`, ordered by `preference` and the user's budget policy. **Use My Models** triggers this for every slot at once.

Users will learn which models are good at what, and the UI should help them do that. The embedding model that builds a good vector index is not the one the planner would guess. Let them pin it.

### Budgets

Cost awareness is a first-class feature, not a settings page.

- **Meters.** Per-provider and per-model token and cost counters for today, this week, and this project. Local models show time and energy instead of dollars.
- **Estimate before run.** Every run shows an estimated token count and cost per step, summed, before the user confirms. Estimates improve from recorded runs of the same recipe.
- **Caps.** Daily and per-run spending caps per provider. A run that would exceed a cap pauses and asks.
- **Policies.** Named rules that feed step 3 of resolution: "drafts on local, final pass on the best cloud model," "never send sources to the cloud," "stay under $2 per run."

### Swapping a model on one step

In Flow View, every AI step card shows its model, estimated tokens, estimated cost, and a dropdown of installed models that satisfy the slot. Changing it pins it. Each step also exposes its provider, prompt version, context, allowed tools, and output schema. Swapping the model never rewires the graph.

### Deterministic versus agentic steps

Both are supported, visually distinct. A deterministic step always runs a particular function. An agentic step can reason, choose from an approved set of tools, and decide what to do next.

The orchestrator has two phases:

1. **Plan.** Translate the goal into a typed workflow, validate dependencies, resolve model and tool requirements, request permission for sensitive actions.
2. **Execute.** Run the approved graph with bounded retries, conditional branches, checkpoints, cancellation, and saved intermediate outputs.

The AI may propose an entirely new workflow. The runtime still enforces what it may execute.

---

## 6. Prompts are versioned, testable resources

A prompt is not a string buried in a node. It is a file with revisions, test inputs, an output contract (JSON Schema), and results across models.

Every run records which prompt version, model, tool versions, inputs, and parameters produced each artifact. That gives reproducibility and selective regeneration: changing the narration script reruns the speech step, not the extraction, analysis, or slide steps.

One subtle feature: compare two outputs side by side and pick the better one, without the user needing to know what A/B testing is.

### The Prompt Stack

Recipe prompts are only the top layer. Underneath them is a small set of system prompts that make the engine work, and those must be just as visible, versioned, and testable. Three tiers:

1. **Planner prompt.** Turns a goal into a proposed graph. Defines what a step is, which capabilities exist, what it may not do (choose providers, grant permissions, invent tools), and the output schema for a plan.
2. **Step envelope.** Every agentic step runs inside the same wrapper: role, input contract, allowed tools, output schema, stop conditions, retry rules, and budget. Recipe authors write only the middle. The envelope is what makes steps from different recipes behave consistently.
3. **Tool and launch protocol.** How a sub-agent is started, what context it is handed, how it reports results, and how it is told to stop. Also the wording used when the app asks the user for permission.

All three live in the same prompt system as recipe prompts: file-backed, revisioned, diffable, with test inputs. A recipe may override the envelope or the launch protocol for its own steps. The Workbench shows the full assembled prompt for any step, tier by tier, so nothing is hidden.

---

## 7. Technical architecture

**Tauri 2 + React + TypeScript, with a Rust execution core.**

```
Tauri desktop shell (React, TypeScript, Vite, React Flow)
        │
Rust runtime (Tokio)
  ├── Recipe executor (plan, validate, schedule, checkpoint)
  ├── Model adapter layer (OpenAI-compatible HTTP, Ollama native, Swift bridge)
  ├── Tool and Skill registry (built-in, MCP via rmcp, Agent Skills, sandboxed extensions)
  ├── Knowledge store (SQLite + FTS5 + sqlite-vec)
  └── Artifact store (SQLite metadata + files on disk)
        │
Providers: Ollama / LM Studio / llama-server · Kokoro · whisper-rs · MCP servers · optional cloud
Outputs:   notes · slides · diagrams · audio · data files · reports
```

### Stack choices (verified versions, Oct 2026)

| Layer | Choice | Notes |
|---|---|---|
| Desktop shell | **Tauri 2.12.x** (2.12.3 current; MSRV Rust 1.90) | Sidecars via `bundle.externalBin` + `Command.sidecar()`, gated by capability files. No Tauri 3 release or timeline; the `3.0.0-alpha` tags are the optional CEF runtime, not core. |
| Frontend | React, TypeScript, Vite | Familiar, flexible |
| Graph editor | **@xyflow/react 12.12** | Custom nodes and edges, sub-flows via `parentId`. DOM-based: fine to a few hundred nodes, which covers recipes. |
| Runtime | Rust, Tokio | Scheduling, processes, checkpoints |
| MCP client | **rmcp 3.7** (official SDK) | Implements 2026-07-28, back-compat with 2025-11-25; stdio and streamable HTTP |
| LLM client | `async-openai` 0.42 (custom base URL) + `ollama-rs` 0.3 | One adapter for all OpenAI-compatible local servers |
| Storage | SQLite + local files | Offline-first, easy backup |
| Vector search | **sqlite-vec** (pre-1.0, exact KNN in stable; ANN only in alpha) | Fine for a personal corpus of tens of thousands of chunks. Move to usearch or LanceDB above roughly 500k vectors. **sqlite-vss is deprecated; do not use.** |
| Local LLMs | Ollama first | See section 5 |
| Speech out | `AVSpeechSynthesizer` day one, then Kokoro via `mlx-audio` (Python sidecar) or `mlx-audio-swift` / Core ML port (no Python). Chatterbox optional for cloning. | Kokoro: Apache-2.0, 54 voices, clean English, weak non-English. Speed should be measured locally, not quoted; no independent MLX benchmark exists. |
| Speech in | **whisper-rs 0.16** with `coreml` + `metal` features linked into the Rust backend | No sidecar, no Python, ~100 MB for large-v3-turbo. Optional Swift helper for Apple SpeechAnalyzer on macOS 26+ (free, on-device, beats Whisper small). Parakeet via MLX is the fastest option but Python-only. |
| Diagrams | **mermaid 12.2** rendered in the webview | Fully offline. Requires WKWebView on macOS 14.4+. Avoid headless mermaid-cli (needs Chromium). Export SVG/PNG via canvas. |
| Slides | Structured slide data + HTML renderer; **pptxgenjs 4.0.1** for export | pptxgenjs is lightly maintained but works client-side |
| Plugins | MCP + Agent Skills + sandboxed extensions | Reuse the existing ecosystem |
| Observability | Local run traces; OTel GenAI span export behind an adapter | Conventions are unstable; isolate attribute names |

Build the core executor in-house. Use existing SDKs for models and protocols. Avoid locking every project into LangGraph or another execution framework. LangGraph and Open Agent Spec can later be import/export targets.

### Keeping the install simple

The installer includes the app, SQLite, built-in tools, whisper-rs, and the diagram and slide renderers. Model weights are optional downloads. On first launch, detect Ollama or LM Studio; otherwise offer one-click local model setup or optional cloud configuration.

No Docker. No account. No mandatory cloud server.

---

## 8. Sharing: GitHub for AI recipes

A recipe is an ordinary directory that can be committed to a repository.

```
weekly-business-report/
├── recipe.yaml          # workflow, requirements, model slots, outputs, runtime compat range
├── README.md
├── prompts/
│   ├── summarize.md
│   └── narrate.md
├── skills/
│   └── financial-summary/
│       └── SKILL.md     # Agent Skills format, unchanged
├── schemas/
│   └── report.json
├── assets/
└── tests/
```

Import from a GitHub URL or a packaged `.recipe` archive. Fork, modify, republish. Someone installs a recipe, presses Use My Models, and runs it on what they have.

A recipe never includes the user's private notes, API keys, or generated artifacts. Sharing a method is separate from sharing content.

**No ecosystem importer.** Stillroom uses the Agent Skills format because it is a foundation standard, but it does not connect to ClawHub, Hermes skill stores, or any other registry. Those ecosystems have had repeated prompt-injection and supply-chain incidents. A one-time, human-reviewed port of a few hundred useful skills into shared recipes is a worthwhile side project. An in-app "import from registry" button is not.

**Gallery signals that matter more than likes:** does it work, what does it require (internet, file write, models), and is the publisher trusted. A beginner should not have to read YAML to learn that a recipe sends data to the internet.

---

## 9. Extensibility: nearly anything is a tool

A unified **Capability** abstraction, backed by a native function, local executable, model invocation, MCP tool, HTTP API, or a nested recipe. Each declares typed inputs and outputs, required permissions, and a consistent result shape.

```yaml
id: diagram.render
name: Render Flow Diagram
inputs:
  definition: string
  format: enum[svg, png]
outputs:
  image: file
permissions: []
```

A **Create Tool** wizard for non-developers. MCP servers and extensions for developers.

### Desktop integrations

```
stillroom run weekly-business-report --input ./exports --output ./report
```

Plus deep links and IPC so Raycast, Alfred, Shortcuts, editors, and scripts can trigger recipes. Eventually expose selected recipes as MCP tools so other agent apps can call them.

---

## 10. Four distinctive features

- **Living execution graph.** Watch steps light up as they run. Click any connection to see the data that passed, any tool decision and its rationale, and the resulting artifact. A debugger and a way to learn agentic architecture at once.
- **Compression control.** Instead of "make it shorter" five times: full, concise, card, five words. Regenerate while warning when a necessary distinction was lost.
- **Change propagation.** Edit a fact or concept; the system identifies every dependent artifact, offers to update them, preserves manual edits, and asks before overwriting conflicts.
- **Adaptive emphasis.** Recipes can learn what the user already knows (via quizzes, feedback, or prior runs) and emphasize only what matters. For learning kits this is "teach me what I don't know." For reports it is "only tell me what changed."

For narrated output, the script is a first-class editable artifact: pronunciation, voice, pauses, rate, chapter boundaries, export formats, all offline.

---

## 11. Security and privacy by design

A downloaded recipe must not automatically gain shell access, read arbitrary files, transmit private data, or run embedded scripts.

- Permission boundaries for filesystem paths, network domains, credentials, and destructive operations.
- Dangerous actions require explicit authorization, surfaced in plain language.
- **Local Only** is the default mode. Cloud inference and external integrations are opt-in, and the UI shows whenever data leaves the machine.
- Community recipes: declarative steps allowed by default; executable extensions require review, sandboxing, and a separate grant.
- Tauri's per-window capability files scope what the webview can call. MCP's own guidance on human control and visible tool invocation applies.

---

## 12. Roadmap

Each stage ships a genuinely useful application.

1. **Stillroom 0.1: one exceptional recipe.** Tauri app, local storage, model discovery (Ollama, LM Studio), source ingestion, Knowledge Spine extraction, the Learning Kit recipe end to end: notes, diagrams, basic slides, narrated audio, flashcards, quiz. Editable artifacts with source references. No graph editor exposed yet, but the engine underneath is already recipe-driven. **Highest priority.**
2. **Flow Studio.** Visual graph editor, typed step contracts, execution replay, prompt versions, model slots, selective reruns, nested recipes. Natural-language planner with validated execution. Every successful ad hoc run offers **Save as Recipe**; editing a step in a saved recipe offers **Save as variant**. Budgets and per-step model pinning ship here. Second and third built-in recipes to prove generality (Inbox Triage shipped early, in revision 3; a research brief is next).
3. **Extensions and sharing.** MCP client, Agent Skills, local extension SDK, recipe bundles, GitHub import/export, curated gallery, permission review UI.
4. **Advanced orchestration.** Multi-agent collaboration, A2A, evaluations, loops and complex branches, background execution, broader model and media support, Apple Foundation Models bridge.
5. **General automation platform.** Headless execution, scheduling, CLI and API, OTel export once conventions stabilize, collaboration. Messaging-app triggers (the OpenClaw pattern) belong here if anywhere, not earlier. Same recipe format throughout.

**Biggest scope risk:** building a generic orchestration platform before shipping one recipe people want. Resist it. Make one exceptional recipe on a small, clean engine, then generalize only what proves it needs generalizing.

### What success looks like for 0.1

- A new user goes from adding sources to a usable output without seeing a node editor.
- A curious user opens the graph, sees exactly how one output was made, swaps its model or prompt, and reruns only that step.

Both should feel natural.

---

## 13. Vocabulary

Lead with the friendly term and put the technical term in parentheses: "How this was made (execution graph)," "Related ideas (knowledge graph)," "Source index (vector database)." This is a less technical product, not a dumbed-down one. People should leave Stillroom knowing the words every other tool uses.

| Term | Meaning |
|---|---|
| Project | A goal you are working on |
| Recipe | A reusable process that produces a result |
| Step | One action in a recipe |
| Tool | A capability a step can use |
| Skill | Reusable expertise or instructions (Agent Skills format) |
| Model | The AI provider filling a capability slot |
| Artifact | A document, diagram, audio file, data file, or other result |
| Run | One recorded execution of a recipe |

---

## 14. The name

**Stillroom.** Chosen from roughly 30 candidates generated across eight naming techniques (craft metaphors, verbs, foreign words, truncations, compounds, portmanteaus, coined words, CLI-friendliness) and collision-checked by web search on 2026-10-26.

**Why it fits.** A stillroom is where the recipe book lived and where things were actually made from it, privately, in the house. Recipes were written to be handed on. The alchemy-plus-kitchen double meaning supports "open the door and watch it happen." Two syllables, spells itself, works as a CLI (`stillroom run`), and sits naturally beside Project, Recipe, Step, Artifact, and Run.

**Collisions found.** One tiny v0.1.0 Go side project (its CLI is `still`, not `stillroom`). Trademarks exist in spirits, soap, and hospitality, none in software. No `.com`/`.ai` software product surfaced. Not yet formally cleared for trademark or domain; do that before any public release.

**Runners-up:** Concoct (verb, clean), Tesser (from Tessera and *A Wrinkle in Time*; crates.io and PyPI collisions), Capstan (sailing; dormant OSv CLI), Simmer (clean, but reads as a variant of Sim Studio, a funded competitor).

**Rejected with reasons:** Thread, Mosaic, Loom, Hearth, Jig, Heddle, Athanor, Galley, Windlass, Telos, Bottega, Muster, Lumo, Backstage, Sidecar. Each has a direct AI or developer-tool collision.

**Taglines:** "Describe the goal. Get the result. Open the door." Alternate: "Every result has a recipe."

---

## 15. The vision

Most agent builders start with a blank graph and ask what nodes you want to connect. Stillroom starts by asking what you want to accomplish, produces something useful, then lets you inspect and modify the system that produced it.

Two reusable layers, and one pattern:

- **Recipe Engine.** Coordinate tools, models, agents, loops, human checkpoints, and execution. Every recipe uses it.
- **Artifact Engine.** Show and edit whatever a recipe produces, chosen by output type and shape (text, tables, slides, diagrams, audio, data), never by recipe name.
- **The Knowledge Spine pattern.** A recipe may convert its sources once into a structured representation and derive every output from it. The Learning Kit does this. It is a pattern a recipe can follow, not a layer the product imposes: an inbox-triage recipe has no spine and loses nothing.

That is what lets the design grow well beyond the showcase without forcing general-purpose workflow complexity onto first users.

The output is the entry point. The orchestration is the discoverable, editable mechanism behind it.

---

## Appendix: corrections made to the original draft

- Reframed from "learning and creation studio" to a general goal-first workflow platform; Learning Kit is the first recipe, not the product.
- MCP 2026-07-28, Agent Skills donation (Oct 22, 2026), and A2A 1.0 claims all **confirmed**; added specifics (stateless core, extensions, deprecations, AAIF governance, A2A 1.0.1 current).
- OpenTelemetry GenAI conventions are **not stable**; moved behind an adapter and out of the "Early" tier's critical path.
- FSRS-5 corrected to **FSRS-6**.
- **sqlite-vss removed** (deprecated); sqlite-vec chosen with its pre-1.0 caveat.
- Ollama now runs **MLX natively** on Apple Silicon (default since v0.40, Sep 2026); "eventually MLX-based integrations" is no longer a separate roadmap item.
- Apple Foundation Models noted as Swift-only, needs a bridge.
- Speech-to-text expanded: whisper-rs as the bundled pick, Apple SpeechAnalyzer and Parakeet as options.
- Kokoro speed claims replaced with "measure locally"; packaging note about the Python sidecar added.
- Tauri pinned to 2.12.x; the Tauri 3 alpha tags clarified as the CEF runtime.
- Competitor table updated: Flowise, Relay.app, OpenAI Agent Builder removed as live; ComfyUI App Mode and Claude Cowork added as the closest threats.
- Added the honest statement that no portable agent-workflow standard exists, with Oracle Open Agent Spec as the nearest candidate and a possible export target.
- Added naming research and the chosen name.

Revision 2, same day, after review:

- Model selection rewritten: pinned > recommended > capability resolve, decided before the run, never by the planner. Added Budgets (meters, estimates, caps, policies) and the per-step swap UI.
- Added the Prompt Stack (planner prompt, step envelope, tool and launch protocol) as versioned, visible system prompts.
- Save as Recipe / Save as variant added to phase 2.
- No ecosystem importer; curated one-time port instead.
- Vocabulary rule flipped: friendly term first, technical term in parentheses.
- Haystack and mozilla.ai checked and ruled out as workflow standards.
- Hermes Agent and OpenClaw added to the competitor table; messaging triggers deferred to phase 5.

Revision 3, same day, after the first six milestones:

- Generality check. The engine was generic but the surface was learning-shaped: one recipe, learning-specific viewers in core UI, three tools, map and branch unsupported, no agent loop, no permissions enforcement, no human checkpoint. Fixed by building a second, non-learning recipe (Inbox Triage) that forced: `map` and `when` execution, an `agent` step with tool calling, a `review` step (the run pauses; approval is fed back as that step's output), enforced filesystem permissions, generic `fs.list`, `fs.write`, `email.parse`, `email.write_draft`, `list.filter`, `list.table` tools, a viewer registry keyed by output type and value shape, and `table` as an output type.
- Resume semantics. Continuing a run (approval or edit) reuses every step the prior run completed with identical inputs, including non-cacheable ones, so a side effect such as filing an email never repeats.
- The Knowledge Spine demoted from a product layer to a recipe pattern (section 15).
- Flow View v0 (read-only): the recipe as a top-down graph with inputs and results in side lanes, the run's state painted on each node in words, and a details panel with a shared model pin. Sized to the drawing, no pan or zoom. Editing on the graph followed the same day: a draft of the bundle (YAML plus files) edited through forms per node and drag-to-wire, checked live by the runtime, saved to the user recipes folder as a copy or in place. Flow Studio's graph editor and model slots are therefore in place; the planner, Save as Recipe from an ad hoc run, and budgets remain.
