# Practice bots, utility effects and targeting polish — 2026-09-24

Six playtest findings from local practice (native server bots, no account or
wallet). Gameplay balance, protocol and persistence schemas are unchanged
except for one new optional preference field.

## Delivered behavior

- **Bots face their movement.** `server/src/bots.rs` computed yaw as
  `atan2(dx, dz)`, the +Z convention minions and neutrals use, while hero
  models are authored facing -Z and every human client sends
  `atan2(-dx, -dz)`. Remote heroes render the raw server yaw, so bots ran and
  attacked backwards. `hero_yaw_towards` now uses the hero convention for both
  movement and in-range facing; a unit test checks the rendered forward vector.
- **Dash effect.** `game_vfx::UtilityVfx::Dash` spawns a departure ring and
  flash, staggered cyan streak afterimages plus ground sparks along the path,
  and an arrival flash, expanding ring and eight radial sparks. The local hero
  emits it where the accepted dash acknowledgment snaps the transform. Remote
  heroes and bots emit it when their replicated `dash_sequence` advances, and
  their interpolation history is cleared so they reappear at the destination
  instead of sliding there.
- **Haste effect.** `emit_haste_trails` follows every hasted hero and paces
  amber speed streaks by distance travelled (alternating sides for a double
  trail), with an occasional ember and a ground ring pulse every 0.55 s that
  also fires immediately on buff start. The shared particle pool grew from
  128 to 192 slots; all effects work in both the 3D and 2D renderers.
- **Camera distance setting.** `CameraSettings { zoom }` is a persisted
  resource shown as Settings > Camera > Distance (55%–225%, 10% steps). Wheel
  zoom writes back into it, so the menu always shows the live distance and
  the choice survives restarts. Reset graphics restores 100%. The preferences
  file gains an optional `camera_zoom` field; older files keep the default.
- **Face the target in melee.** `resolve_basic_attack` turns the local hero
  toward its target while in reach (eased between strikes, snapped on the
  strike) unless the phone stick is steering or a move order is pending. The
  yaw is sent with the regular transform packet, so other clients see it.
- **Drag lock only selects.** On phones, releasing an ATTACK drag on a unit
  now sets the target without starting the attack or the approach. Tapping or
  holding ATTACK, and the minion/tower category buttons, still attack and
  chase as before.

## Verification

Client and server unit suites, formatting and clippy in this checkout. No
native capture was recorded in this container; the visual tuning of the new
particles (sizes, lifetimes, colors) is meant to be reviewed in a local
practice match.

## Practice sandbox (same day, follow-up)

Playtest asks: a menu to switch the local practice between the standard
roster, an empty map, a target dummy and a configured 1v1; god mode from that
menu; and a K/D/A that did not grow when killing bots.

- **Where it lives.** "Offline" practice is the local `server` process that
  `make practice` starts next to the client, so the sandbox is a
  practice-only server command (`shared::practice::PracticeCommand`, sent as
  `ClientPacket::Practice`) rather than a second, client-side simulation. The
  server ignores it outside `MatchMode::Practice`, from allocated workers and
  from bot addresses.
- **Bot kinds** (`server/src/bots.rs`): `Lane` (the roster), `Duelist` (the
  lane AI at a configured level and budget) and `Dummy { anchor }` (never
  thinks, moves or attacks; 400 HP; after a base respawn it is placed back on
  its anchor facing the human). Only lane bots count toward the team size, so
  dummies never displace the roster; `BotControllers::sandbox` stops the
  automatic refill after "Clear bots" or a duel until "Standard bots" or a
  round reset.
- **Duelist setup.** Level via the ordinary XP path (HP/mana growth included),
  skill points spent ultimate first then Q/W/E into unlocked slots
  (`auto_rank_skills`), gold awarded and spent through the real purchase path
  at the base shop in the class's recommended order (`auto_shop`). Both
  helpers now run for every bot on its think tick, and bots cast every
  unlocked hostile-target slot, so lane bots also shop and use their kit.
- **Client page** (`client/src/practice_sandbox.rs`): a "Practice sandbox"
  button on the pause menu's main page, shown only while a practice match is
  confirmed, opens a page with the god-mode toggle (shared with the debug
  toggle state so the debug HUD never overrides it), the bot buttons, level
  and gold adjusters (1–10, 0–1000 in 100 steps) and "Start 1v1 on mid".
- **K/D/A.** Two new end-to-end server tests (direct join and the draft flow
  the shipped client uses) confirm a human's basic attacks that kill a bot
  produce 1/0 for the human and 0/1 for the bot on the live scoreboard, so
  the reported freeze could not be reproduced from the practice server alone.
  Two hardening changes ship anyway: round statistics now roster anyone who
  joined after the round was drafted (for example bots filled while a durable
  start acknowledgment was pending) before the first hit, and the ledger logs
  `MATCH_METRIC event=unregistered_participant` once per id when a hero hit
  involves a player without a row. If the counter still stays flat, that log
  line on the practice server names the missing participant.
