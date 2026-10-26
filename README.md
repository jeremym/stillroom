# Stillroom

A goal-first, local-first AI workflow studio for people who have never built a workflow.

> Describe the goal. Get the result. Open the door.

Describe what you want to accomplish. Stillroom plans the steps, picks the models and tools, runs the workflow, and hands you real outputs: documents, slides, diagrams, audio, reports. Then, only if you want, open the door and see the machine that made them. Everything it builds is a portable **Recipe** you can edit, rerun, and share.

Local models by default. No Docker, no account, no mandatory cloud.

## Status

Pre-alpha. The architecture and roadmap are in [PLAN.md](PLAN.md).

## Layout

```
schemas/        Recipe format (JSON Schema)
crates/core     Recipe schema types, executor, run traces
crates/models   Model adapters (OpenAI-compatible, Ollama native)
src-tauri/      Tauri 2 desktop shell (Rust)
src/            React + TypeScript frontend
```

## Development

Requires Rust 1.90+, Node 22+, pnpm, and Xcode command line tools on macOS.

```
pnpm install
pnpm tauri dev
```
