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
