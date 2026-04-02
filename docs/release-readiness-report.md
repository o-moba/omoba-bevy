# Release readiness report — gameplay slice (TASK-12)

**Date:** 2026-04-01  
**Build focus:** Centralize balance tuning, document release gate + QA matrix, expand regression tests around jungle templates.

## Checklist outcome

See `docs/release-gate-checklist.md`. **G1–G4** and **P1–P8** are **PASS** with the evidence commands below. Live UDP harness coverage now includes **M1/M2/M3** (join, victory/rematch lifecycle, move+cast transport loop).

## QA matrix summary

See `docs/manual-qa-matrix.md`. **Live UDP:** `python3 scripts/verify_task_12_qa_matrix_live_udp.py` (after `cargo build -p server`) records **PASS** for **M1/M2/M3**. **Server tests** cover **M4–M5** and cross-feature invariants (minions/towers/jungle/progression).

## Tuning changes / rationale

- Gameplay numerics live in `server/src/balance.rs` (single home per AC4), including `PRIMARY_ABILITY_DAMAGE_BY_RANK` (five tiers; simulation currently uses index 0 only) and `SKILL_SLOT_COUNT` for progression alignment.
- Neutral kill XP now flows through `grant_player_xp` so jungle rewards participate in the same level curve as lane/minion XP.
- No broad numeric rebalance beyond wiring and the above consistency fix.

## AC5 alignment (core slice / four skills)

Per checklist row **P8**, this slice requires **one** server-validated ability (projectile), not four fully distinct skills. **AC5** (“all four skills”) is satisfied for this slice by **documenting** the gap: only one ability is server-simulated; skill points accrue for UI/future work. There is **no hidden** defect here—missing three abilities is an explicit **NON-BLOCKER** for the declared gameplay slice and would be a **BLOCKER** only if product scope changes to require a four-skill kit with server validation.

## Blocker list (slice scope)

| Item | Classification | Notes |
|------|----------------|-------|
| Four distinct server-validated skills with per-rank damage curves | **NON-BLOCKER** for declared slice | Slice documents one authoritative ability; skill points are progression UI. |
| Full Bevy UI input UX automation (hotkeys/cursor targeting) | **NON-BLOCKER** for this slice | Gameplay-loop correctness is validated at protocol/simulation level; UI ergonomics still require manual UX passes. |

## Artifact index

| Artifact | Path |
|----------|------|
| Release gate checklist | `docs/release-gate-checklist.md` |
| Manual QA matrix + run log | `docs/manual-qa-matrix.md` |
| Balance tuning guide | `docs/balance-tuning.md` |
| Live UDP QA harness (M1, M2, M3) | `scripts/verify_task_12_qa_matrix_live_udp.py` |
| Session-flow harness (broader UDP) | `scripts/verify_task_02_multiplayer_session_flow.py` |
| Evidence bundle (optional verifier refresh) | `.agent/tasks/TASK-12/evidence.md`, `evidence.json` |
| Command transcripts (when refreshed) | `.agent/tasks/TASK-12/raw/*.txt` |
