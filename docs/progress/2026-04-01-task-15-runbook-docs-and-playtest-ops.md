# Progress: TASK-15 runbook docs and playtest ops

**Date:** 2026-04-01  
**Task ID:** TASK-15  
**Goal:** Documentation-only deliverables so testers can run, verify, and report without reverse-engineering the repo.

## Changes

- Added `README.md` with setup, happy-path pointers to `RUNBOOK.md`, controls summary, and tester doc index.
- Added `docs/playtest-script.md` (timeboxed 10–20 minute checklist).
- Added `docs/bug-report-template.md` and `docs/mvp-scope-and-limitations.md`.
- Added `tasks/MVP-CHECKLIST.md` for MVP-blocking vs deferrable labeling.
- Expanded `RUNBOOK.md` troubleshooting with concrete recovery steps (ports, LAN, firewall, stale processes).
- Linked documentation from `docs/features.md`; logged in `CHANGELOG.md` under Unreleased.

## Checks run

- `cargo build --workspace`
- `cargo test -p client` (5 tests; client has no separate lib target)
- `cargo test -p server` (10 tests)
- `cargo clippy --workspace --all-targets -- -D warnings` — **fails** on pre-existing server lints (`clippy::items_after_test_module`, `clippy::too_many_arguments` in `server/src/main.rs`); doc task does not change production code to fix.

## Risks / follow-ups

- AC1 blind “doc-only” walkthrough should be assigned to a reader with no prior context per the frozen spec verification plan; this session focused on doc completeness and command sanity.
- 2026-04-02 finalize: `.gitignore` updated so `docs/*` / `tasks/*` deliverables are trackable; `docs/features.md` release-gap list aligned with shipped match lifecycle / jungle / neutrals; `RUNBOOK.md` asset troubleshooting line aligned with `client/assets` behavior.
