# Mobile MOBA edge HUD — 2026-09-22

Task: TASK-MOBA-EDGE-HUD-2026-09-21
Branch: codex/ui-release-polish
Baseline:84a6a92; target version:0.21.0-rc.3

The user clarified that protecting only the upper approach moved the obstruction
elsewhere. The supplied Wild Rift image defines the intended mobile composition:
compact edge information, left movement and a right combat fan. The follow-up
explicitly keeps desktop skill cards centered below and moves its map to upper left.
An independent art director translated that reference into OMOBA geometry and style.

## Changes

- Both profiles use an upper-left map, clickable gold and two real quick buys,
  compact local vitals, top-right score/KDA/icon controls and selected-target HP.
- Mobile category attack controls select minions or structures explicitly. Skill
  rank rings, cooldowns and an integrated full-size upgrade mode replace the
  scattered upgrade buttons. Desktop skill cards and keyboard behavior remain.
- The score opens both teams' authoritative current-round statistics. Earned gold
  is tracked separately from spendable balance, across death/reconnect and reset.
- Mobile dash/haste are real server-validated gameplay actions with shared tuning,
  match-bound requests and cooldown replication. Dash clips obstacles and has
  explicit reconciliation/transform acknowledgment; haste affects predicted and
  authoritative movement for the same bounded duration.
- Shop inventory is available inside the modal, and its existing receipt/retry
  workflow also handles quick purchases. Menu icons retain chat/reactions and
  expose the controls guide through the game menu.

## Proof and limits

Frozen acceptance criteria, art direction, per-area check logs, screenshots,
layout data and the fresh independent verdict live under
.agent/tasks/TASK-MOBA-EDGE-HUD-2026-09-21/ in the dedicated worktree. The final
handoff/evidence records exact command totals and tested viewport coverage.
Native phone captures are desktop development previews. Repeatable scoreboard
and target fixtures are labeled and supplemented by real behavior/network tests.
Physical-device usability/performance and release-platform checks remain separate.
No dependency, credential, production service or release deployment changed.

Independent lifecycle verification also found and fixed deliberate leave/rejoin
reusing a live scoreboard identity. Fresh guest admission now gets a new round
participant, preserving the retired row and any delayed damage attribution.
Ordinary session reconnect retains progression, equipment and cooldowns. Signed
profiles retain one participant per round: after a deliberate leave, a new hero
selection waits for a new round instead of silently rewriting the old identity.
