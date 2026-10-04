# Playtest iteration commit and push — 2026-10-04

Version 0.39.0 combines the locally implemented 0.37 playtest work, 0.38
Adventurer/touch reversal, and 0.39 configurable lane defenses. The feature
branch is `feat/playtest-round-two`, based on `4a7b65a` (0.36.0).

The delivery scope is commit/push of this feature branch. It does not merge
main, deploy the server or upload TestFlight. The assembled protocol is 8;
protocol 6/7 and pre-commit status in earlier step notes describe those steps'
historical snapshots. Both game host and client require a coordinated update.

[REFACTORING.md](../REFACTORING.md#post-programme-architecture-follow-ups--2026-10-04)
is the single current TODO, with checked implemented foundations and open
acceptance boundaries. The September scale report remains historical. The
changelog records user-visible improvements and the current delivery limits.

The review's Bluff/Brittle discrepancy, fully recipe-driven mobile metadata,
actor/controller/transport separation, unattended worker capacity, configurable
brush, catalogue order and focused module/UI extraction are future work. This
commit does not silently implement those refactors or declare their risks fixed.

Existing evidence is under `.agent/tasks/PLAYTEST-ROUND2-20261004/`,
`DAGGER-CLASS-20261004/`, `MOVEMENT-REVERSAL-20261004/`, and
`LANE-DEFENSE-20261004/`. Commit-gate commands and remote verification are kept
in `.agent/tasks/ITERATION-COMMIT-20261004/`. Physical iPhone reconnection,
120 Hz and match balance still need playtesting; no new UI capture matrix is
part of committing this already-inspected iteration.

## Commit verification

- `make check`: PASS — formatting, workspace/all-target strict Clippy, client
  without QA, 1,515 Rust tests and 201 Python tests. The default gate leaves
  39 opt-in/database/reporting tests ignored; no disposable PostgreSQL run was
  performed for this delivery.
- Current native workspace binaries: PASS, built from this checkout in cache A.
- Serial real-server harness: all 47 tests pass across the complete run and
  repaired-target reruns. The complete run exposed five failures in two
  targets; current `combat_actions` (2/2) and `gameplay` (8/8) reruns pass.
  Other targets remained unchanged and passed, including map variants,
  populated framed snapshots, legacy UDP compatibility, jungle, matchmaking
  and release lifecycle. Final harness strict Clippy and workspace formatting
  also pass.
- Gate repairs are limited to QA-only brush identifiers and test fixtures:
  current framed transport, ordinary bot navigation, one complete public
  structure list instead of duplicate obstacles, and a speed probe whose
  corridor is verified against terrain and live towers. Existing gameplay
  thresholds and time budgets remain intact.
- Independent documentation and final harness source reviews: PASS. Device
  and release follow-ups remain in the canonical refactoring/release tracker.
