# 2D controls and camera follow — 2026-07-28

Task: `TASK-2D-CONTROLS-FOLLOW-01`

## Outcome

The apparent movement freeze was an input-state coupling rather than a server
or collision failure. Right-click or Alt toggled `CameraState::locked`, and the
ground-click handler rejected every movement command while that flag was
false. In `sprite2d`, movement commands now remain available in both follow and
free-pan states. Right-click and Alt no longer toggle the 2D camera or capture
the cursor; their established 3D behavior is unchanged.

`Y` is the explicit follow toggle. When a minimap focus override exists, `Y`
clears it and keeps follow enabled so one key press returns to the local hero.
`Space` continues to force the same return-to-hero behavior. Camera input,
follow update, and movement handling are ordered deterministically within each
frame.

Player nameplates now use a smaller base size and shrink further based on the
display-name length. Tests bound estimated label height and width against each
hero's configured world height, including Orchard Comet Centaur.

## Gameplay impact

- No movement speed, collision radius, server authority, packet, or map logic
  changed.
- Free camera can be used while continuing to issue movement orders in 2D.
- The local player can always recover hero follow with `Y` or force it with
  `Space`.
- Labels no longer visually overpower the character sprites.

## Verification

The proof-loop command results and fresh acceptance-criterion verdict are in
`.agent/tasks/TASK-2D-CONTROLS-FOLLOW-01/`.
