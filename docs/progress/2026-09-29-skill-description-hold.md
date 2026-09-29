# Skill description hold — 0.30.1

- [x] Require a continuous 1.5-second hold of one skill; ignore hover and short presses.
- [x] Reset inspection on release, skill/input-owner changes and gameplay interruption.
- [x] Use the same threshold for hotbar, keyboard, controller and stationary touch.
- [x] Hide the standard-kit explanation panel outside inspection; remove redundant
  successful-cast text. Keep rejection messages, aiming and cast timing intact.
- [x] Regression coverage for timing, rapid taps, switching, attacks, menu blocking,
  touch inspection release and drag aiming.
- [x] Final gate: 1,331 Rust and 192 Python tests passed; 39 existing
  environment-dependent Rust tests remain ignored. Both Clippy configurations pass.
- [x] Native client/server build and three affected English 1280×720 states pass:
  short hold hides descriptions, long hold shows them, release hides them.
  Screenshots were inspected; input was scripted rather than manual.

Implementation: `combat/inspection.rs` owns presentation timing; it cannot issue
casts. The existing skill card and standard status panel read its result. Touch
retains its existing gesture ownership and no-cast inspection release.

Verification budget: English, existing 1280×720 desktop profile, three states.
No physical mobile or additional locale checks. Evidence:
`.agent/tasks/UX-SKILL-HOLD-2026-09-29/`.
