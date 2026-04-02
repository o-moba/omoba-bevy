# Manual multiplayer QA matrix — gameplay slice

## Matrix (scenarios × areas)

| ID | Scenario | Session / join | Match lifecycle | Combat / ability | Minions / towers | Jungle | Progression HUD | Expected |
|----|----------|----------------|-----------------|------------------|------------------|--------|-----------------|----------|
| M1 | Two local clients, same machine (`make start` or equivalent) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | Both see world updates; no desync on team/id |
| M2 | Match to victory (destroy enemy base) | | ✓ | ✓ | ✓ | | ✓ | Winner correct; rematch or countdown behaves per design |
| M3 | Cast ability on minion, structure, neutral, enemy player | | ✓ | ✓ | ✓ | ✓ | | Mana decreases; hit registers when valid |
| M4 | Clear each neutral camp type once | | | ✓ | | ✓ | ✓ | Gold/XP matches expectation; respawn after cooldown |
| M5 | Level up twice in one match | | | ✓ | ✓ | ✓ | ✓ | Server level/skill_points match HUD |

## Run log

**Session:** 2026-04-01 — TASK-12 (live UDP harness M1/M2/M3 + server tests M4/M5).

| ID | Executor | Result | Notes |
|----|----------|--------|-------|
| M1 | `python3 scripts/verify_task_12_qa_matrix_live_udp.py` | **PASS** | Two UDP clients on `127.0.0.1:4012`; distinct `your_id`; each snapshot lists both players with consistent teams/spawns. Requires `cargo build -p server` first. |
| M2 | `python3 scripts/verify_task_12_qa_matrix_live_udp.py` | **PASS** | Two green UDP clients repeatedly cast `TargetKind::Structure` at the blue base tower until `game_state=victory` (winner=green), then observe auto-rematch reset to `running` with restored base HP. |
| M3 | `python3 scripts/verify_task_12_qa_matrix_live_udp.py` | **PASS** | Script verifies movement (`transform` reflected in snapshots), then cast path (mana drop + enemy HP drop) over live UDP. This validates transport-level move+target+cast loop for the slice. |
| M4 | Automated (server tests) | **PASS** | `neutral_kills_award_rewards_and_respawn_on_cooldown`, `neutral_camps_spawn_alive_with_distinct_templates`, leash test. |
| M5 | Automated (server tests) | **PASS** | `progression_levels_up_and_scales_stats`, `respawn_restores_scaled_maximums`. |

**Summary:** M1–M3 are executed and recorded via the live UDP script (fresh server per scenario). M4–M5 remain server `cargo test` evidence and are documented as simulation-backed checks.

## Major loops vs AC3

**Validated with live multiplayer transport (two UDP endpoints):** session/join (M1), match lifecycle victory→rematch (M2), and move+cast/mana/hit protocol loop (M3). **Validated in server simulation only:** jungle rewards/respawn/leash (M4), progression/respawn (M5), minion waves + tower fire (`neutrals_do_not_break_minion_waves_or_tower_attacks`). **Explicit note:** Bevy UI hotkeys/cursor targeting remain manual UX checks; gameplay-loop correctness is covered by multiplayer protocol evidence plus simulation tests.
