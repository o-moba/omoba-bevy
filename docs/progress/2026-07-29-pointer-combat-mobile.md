# Pointer-first desktop/mobile combat — 2026-07-29

Task: `TASK-POINTER-COMBAT-MOBILE-01`

## Outcome

The apparent inert skills were caused by an input and validation gap rather
than missing server combat. Pointer target acquisition only listened to middle
mouse and searched around a projected ground point with a fixed world radius.
Normal left clicks always became movement, touch had no world action path, and
the local cooldown began before the server could reject an out-of-range cast.

Desktop primary clicks and mobile taps now resolve the exact hostile actor in
screen space. Living enemy heroes, minions, neutrals, towers, and bases use
48–68 logical-pixel hit radii independent of camera zoom. A target gesture
selects the authoritative entity, exposes the gold marker/HUD target, consumes
the movement gesture, and requests Q. Empty-ground gestures move on both mouse
and touch. Tab and selection-only middle-click remain supported.

All keyboard and hotbar slots queue through one target-aware path. Unit-target
abilities approach a moving out-of-range target and emit one network command
when the shared scaled range is satisfied. The local cooldown starts only at
that emission point. Ground movement, target death/removal, insufficient mana,
locked abilities, and explicit target clear end pending work safely. Self-cast
abilities remain immediate and target-free.

## Scope

No server ability numbers, damage, mana cost, cooldown, class kit, projectile,
collision, or target-authority rules changed. The client reuses shared scaling
functions and the existing `NetworkCommand::Cast` protocol.

## Verification

Final command output, acceptance evidence, and the fresh verdict are stored in
`.agent/tasks/TASK-POINTER-COMBAT-MOBILE-01/`.
