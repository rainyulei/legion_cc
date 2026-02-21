# Changelog

## [0.1.3-beta] - 2026-02-21

First public beta release.

### Features

- **Teams & Roles system** — define team compositions (Tech Lead + Engineer + QA) with custom prompts, assign teams to tickets via `--team` flag
- **MiniMax provider** — OpenAI-compatible integration for MiniMax coding models (M2.5, M2.1, M2)
- **GitHub Copilot Codex support** — `gpt-5.2-codex` model auto-routes through Responses API adapter; single Copilot provider handles all model formats
- **Verification checkpoints** — `/split-tickets` methodology now includes checkpoint tickets for build/test/lint gates between functional modules
- **Bracketed paste** — pasted text appears instantly in PTY (no more character-by-character display)
- **Popup key scoping** — Esc and other keys properly scoped to popup context (no longer leaks to PTY)
- **Paste in popup inputs** — API key, session name, and retry form inputs now accept pasted text
- **Text wrapping in popups** — long content (context, criteria, activity descriptions) word-wraps correctly
- **Copy hint** — squad mode footer shows "Shift+Drag: Copy" for terminal text selection
- **Makefile** — `make build`, `make pkg`, `make install` for streamlined builds

### Bug Fixes

- Panel click sometimes made j/k ticket selection stop working
- Error/criteria content in ticket detail popup couldn't be scrolled
- File diff popup title truncated for long paths
- Board detail popup Esc key conflicted with Claude Code's native Esc

### Architecture

- **OpenAI Responses API adapter** — bidirectional transform (Anthropic <-> Responses API) for providers using the `/v1/responses` endpoint
- **Effective format routing** — proxy server uses model-aware format detection instead of static `api_format` field
- **CLI refactor** — simplified commands (`legion`, `legion squad`, `legion switch`)

## [0.1.0] - 2026-02-13

Initial internal release.

- Single and squad mode TUI
- Proxy server with Anthropic/OpenAI/Copilot routing
- Orchestration engine with DAG scheduler
- Git worktree isolation per worker
- Auto-merge pipeline
- Ralph Loop retry mechanism
- Session management with SQLite persistence
- Per-pane provider/model configuration
- Task board with ticket lifecycle tracking
