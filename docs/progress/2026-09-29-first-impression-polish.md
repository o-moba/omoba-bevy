# First impression UI polish — 2026-09-29

## Scope and result

Home, Hero Select and Settings now use a more consistent Verdant hierarchy. This iteration prioritizes a clear character presentation and controls that fit the viewport; it does not redesign the in-match HUD or matchmaking flow.

- Home has a single screen-owned header, with brand and utility controls inset from the ornament. The build label and footer are inside the frame. A compact status pill remains readable against the scene. Touch devices no longer display a second floating utility bar over the header. Tablet hit targets retain at least 44 px after scaling; party invitations occupy a bounded slot instead of overlapping the header.
- Hero Select keeps class choice, illustrated avatars, the selected hero and the primary action visible together. Phone uses three compact columns; tablet and desktop use a spacious roster. The roster scrolls independently. Desktop/tablet account controls remain below included avatars; compact phones retain their existing account entry from Home.
- Settings uses icon navigation, section cards, gold headings, consistent control columns, volume/lighting sliders and switches. The close control stays outside the scrolling body. Mobile Exit behavior and all three languages are retained.
- Background transitions complete even when a capture window is unfocused. Decorative animation still pauses while unfocused. Sliders distinguish horizontal adjustment from vertical scrolling.

## Verification

Task specification and raw evidence: `.agent/tasks/UI-FIRST-IMPRESSION-2026-09-29/` in the primary checkout. The task uses the dedicated `codex/first-impression-polish` worktree and shared build cache A only.

The client library suite passes **788 tests**, with one existing ignored test. The locked client/server build, client Clippy with warnings denied, formatting and diff checks pass. The shell capture harness covers desktop 1280×720, phone 844×390 and tablet 1180×820. English captures and Russian/Chinese samples are checked visually for clipping and font fallback. The shell harness exercises the Home, picker and menu navigation and the settings scroll-to-end path. An initial sandboxed run could not bind localhost sockets; the unrestricted rerun passes.

Independent verification caught simultaneous parent scrolling during a slightly diagonal slider drag. A combined slider/scroll regression reproduced the movement from 40 to 48 px, then passed after fixing gesture ownership. A gesture starting on a slider now keeps the first recognized axis: horizontal adjustment or vertical scrolling. Generic list scrolling is unchanged.

These are macOS-rendered viewport simulations. Physical iPhone/iPad/Android interaction and native packaging remain device validation steps; this change does not claim a new TestFlight upload.

## Release

Workspace version: **0.28.4**. No production dependency, server protocol, infrastructure or signing changes.
