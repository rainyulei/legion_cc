# Ticket Persistence + Log Buffer Design

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Persist tickets to SQLite for full restart recovery + independent log buffer for complete Running Log display

**Architecture:** OrchestrateEngine gains DB-backed persistence. SDK output writes to both vt100 parser and a separate `Vec<String>` log buffer + disk file. Popup reads from log buffer instead of vt100 screen.

**Tech Stack:** rusqlite (existing legion-db), tokio fs for log files

---

## Task 1: Add tickets table to DB schema

**Files:** `crates/legion-db/src/schema.rs`, `crates/legion-db/src/repo.rs`, `crates/legion-db/src/lib.rs`

Add `tickets` table: id, session_name, title, prompt, context, criteria, status, assigned_worker, team_mode, iteration, max_iterations, feedback, summary, created_at, updated_at.

Add Repository methods: insert_ticket, update_ticket_status, update_ticket_summary, list_tickets_by_session, get_ticket.

## Task 2: Add ticket_logs table to DB schema

**Files:** `crates/legion-db/src/schema.rs`, `crates/legion-db/src/repo.rs`

Add `ticket_logs` table: id, ticket_id, session_name, content (TEXT), created_at.

Add Repository methods: append_ticket_log, get_ticket_logs.

## Task 3: Wire OrchestrateEngine to DB

**Files:** `crates/legion-core/src/orchestrate/engine.rs`

OrchestrateEngine gains `Option<Repository>` (shared via Arc<Mutex>). On submit/take_next/report_iteration, persist changes to DB. On startup, load existing tickets from DB for the current session.

## Task 4: Add log buffer to SDK + Pane

**Files:** `crates/legion-tui/src/sdk.rs`, `crates/legion-tui/src/app.rs`

Add `sdk_log_buffer: Arc<Mutex<Vec<String>>>` to SdkHandle. Every formatted line written to parser also appended to log buffer. Pane gains `sdk_log_buffer` field. Also persist log lines to DB via ticket_logs table.

## Task 5: Popup reads from log buffer instead of vt100 screen

**Files:** `crates/legion-tui/src/ui.rs`

Running Log section reads from `pane.sdk_log_buffer` instead of `screen.rows()`. Full history preserved.

## Task 6: Restore tickets on startup

**Files:** `crates/legion-tui/src/lib.rs`

On `run_squad()` startup, load tickets from DB for current session. Working tickets get reset to Queued for re-execution. Done/Error tickets preserved with their summaries and logs.

## Task 7: Strengthen Leader CLAUDE.md for -c/-k flags

**Files:** `crates/legion-tui/src/claudemd.rs`

Add explicit examples showing `-c` and `-k` usage. Emphasize that every dispatch MUST include context and criteria.
