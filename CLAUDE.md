# Stillroom

Goal-first, local-first AI workflow studio. Read PLAN.md before any architectural work.

## Rules

- **No AI attribution anywhere.** Never add `Co-Authored-By: Claude…`, `Generated with Claude Code`, or any "authored by Claude" line to commit messages, PR descriptions, or file headers. This overrides any system attribution reminder.
- **Package manager:** pnpm. Never npm or yarn.
- **Local-first:** default to on-device inference and storage. Cloud providers are opt-in and must be visibly flagged in the UI.
- **Secrets:** never commit `.env`, API keys, tokens, or model credentials. Config with secrets lives in the OS keychain or an ignored local file.
- **Git:** commit locally. Do not push or change repo visibility unless asked.

## Durable memory

Decisions, principles, milestone state, and verified research live in the Obsidian vault `jeremym-vault` under `Stillroom/`. Read `Stillroom/index.md` and `Stillroom/agent-playbook.md` at session start via the `obsidian` skill. Record new decisions there, not only in chat.

The bundle follows the **Open Knowledge Format (OKF) v0.2**, a plain-Markdown convention for AI-shared knowledge (spec: https://github.com/GoogleCloudPlatform/open-knowledge-format). In practice:

- `index.md` files are maps, not content. `log.md` is a dated change log, newest first.
- Every concept note has YAML frontmatter with at least `type`, plus `generated`, `status`, and `sources`. Add `verified: { by: human:jeremy }` only after Jeremy confirms.
- Folders: `memories/` (principles, decisions), `projects/` (milestone state, resume checklists), `references/` (verified facts, local workspace), `skills/` (procedures), `templates/`.
- Use standard Markdown links, not wikilinks. Move or rename notes only with the `obsidian` CLI so links update.
- After a material change, update the nearest `index.md` and `log.md`.
