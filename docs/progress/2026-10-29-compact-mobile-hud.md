# Compact mobile HUD — 2026-10-29

The owner supplied OMOBA and annotated Wild Rift screenshots and requested a
smaller, more comfortable battle HUD with more visible world. Work is isolated
on `fix/compact-mobile-hud`, based on dev `7ace97c`.

The change reduces combat artwork while retaining independent capture regions,
moves Recall and movement utilities to the bottom, lightens/repositions the
joystick, and compacts the score, ally portraits, service controls and shop art.
It does not change gameplay, menu scale, desktop controls, region routing or any
server/deployment settings. The shared Verdant token handoff is untouched.

Evidence is in `.agent/tasks/MOBILE-HUD-COMPACT-20261029/`. Validation is in
progress. One English phone preview is planned at 844×384 logical pixels, matching
the supplied 1280×582 aspect. The phone was disconnected when work began; physical
installation/testing must be recorded separately from a rendered preview.
