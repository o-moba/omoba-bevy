# Release gate checklist — gameplay slice

**Scope:** Prototype MOBA-style slice: UDP session, match lifecycle, lanes, minions, towers, one server-validated player ability, jungle neutrals, progression (XP / level / skill points).

**How to use:** Each row is **PASS** only if the criterion is true for the current build. **FAIL** blocks calling the slice release-ready. **PARTIAL** means documented gaps remain (see release readiness report).

| # | Criterion | Result (PASS / FAIL / PARTIAL) | Notes |
|---|-----------|--------------------------------|-------|
| G1 | Slice scope is defined (this document + `docs/release-readiness-report.md`). | **PASS** | Scope repeated in both documents. |
| G2 | `cargo build` succeeds for the workspace. | **PASS** | See `.agent/tasks/TASK-12/raw/build.txt` (evidence refresh). |
| G3 | `cargo test` succeeds (client + server). | **PASS** | See `raw/test-unit.txt`, `raw/test-integration.txt`. |
| G4 | English release artifacts exist: this checklist, manual QA matrix + run log, readiness report, balance tuning doc. | **PASS** | All under `docs/`. |
| P1 | Two or more clients can connect to a running server and receive snapshots. | **PASS** | `scripts/verify_task_12_qa_matrix_live_udp.py` (M1) plus `scripts/verify_task_02_multiplayer_session_flow.py` for broader session flows; `make start` for full Bevy clients. |
| P2 | Match flows: lobby → running → victory/rematch without server restart. | **PASS** | `scripts/verify_task_12_qa_matrix_live_udp.py` (M2) drives live base destruction to `victory` and observes auto-rematch reset to `running` in the same server process. |
| P3 | Players can move, select targets, and cast the authorized ability while mana/cooldown rules hold. | **PASS** | `scripts/verify_task_12_qa_matrix_live_udp.py` (M3) verifies live UDP movement (`transform` reflected in snapshots) and cast effects (mana spend + HP drop). Bevy UI input UX remains manual but does not block loop correctness for this slice. |
| P4 | Minion waves spawn and apply lane pressure; towers engage valid targets. | **PASS** | `neutrals_do_not_break_minion_waves_or_tower_attacks` exercises waves + tower shots alongside jungle aggro. |
| P5 | All three jungle camp types can be engaged; rewards and respawn behave consistently with `server/src/balance.rs`. | **PASS** | Template + kill/respawn + leash tests; `balance::tests` guard constant coherence. |
| P6 | XP and leveling update on server and appear in client HUD; skill points increment on level-up. | **PASS** | `progression_levels_up_and_scales_stats`, `respawn_restores_scaled_maximums`; HUD sync documented in `docs/features.md`. |
| P7 | Every known defect affecting slice goals is listed in the readiness report as **BLOCKER** or **NON-BLOCKER** (no ambiguous “maybe”). | **PASS** | Readiness report blocker table; open items have explicit classification. |
| P8 | Four distinct skills with full server validation and per-rank tuning are **not** required for this slice; gap is explicit in readiness materials if absent. | **PASS** | One server-validated ability; four-skill kit called out as out-of-scope for slice (see readiness report). |

**Release-ready definition:** Rows G1–G4 and P1–P7 are **PASS**, and there are **no open BLOCKER** items for the scoped slice. If any row is **FAIL** or **PARTIAL**, the slice is **not** release-ready until the report records the exception and owner decision.
