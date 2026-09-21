# UI composition and release polish — 2026-09-21

Task: `TASK-UI-RELEASE-POLISH-2026-09-21`
Branch: `codex/ui-release-polish`
Version: `0.21.0-rc.2`

The user requested an art-directed interface pass across the game and specifically
identified upper HUD panels obscuring opponents entering from the north. The work
uses a frozen acceptance specification and independent art-direction and review
passes in `.agent/tasks/TASK-UI-RELEASE-POLISH-2026-09-21/`.

## Changes

- A compact desktop tactical dock holds the minimap, hero resources, abilities,
  and equipment. Objective/target/buff status aligns above the abilities; empty
  buff text consumes no space. The central 30% of the upper 30% remains clear.
- Phone status and social actions occupy the left corner beside the minimap;
  objective status sits between the lower thumb controls. The attack fan and
  stick retain their input geometry.
- One Verdant palette, restrained champagne accents, jade primary actions,
  consistent control edges, and shared typography tie the frontend together.
  Home gives identity, the selected hero, and matchmaking a composed stage.
- Phone menus compensate for shell scale to preserve readable text and touch
  targets. Pause, career, server, and help modals restore their actual phone scale.
- Help copy is shorter. Settings rows align, and the main game menu sizes to its
  content. Account forms have appropriate widths and a profile-first hierarchy.
  Offline shell settings remain available; phone utility buttons yield to the shop
  so they cannot cover its Close action. A fresh verifier found and prompted a fix
  for Home Help: an explicit request now shows the guide before admission, and
  Dismiss/Escape restores the shell; automated first-match help stays gated.
- Native QA now follows the current shell admission flow and records northern
  sightline clearance, HUD text containment, and panel bounds. Shell capture
  includes pause/settings/server forms and career capture includes website/device screens.
  Essential card and collection actions are checked against the viewport after phone
  text and touch-target adaptation.

## Verification and limits

Current command logs, screen captures, layout measurements, and the independent
verdict are retained in the task evidence directory. The capture matrix covers
1280×720 desktop and 844×390 landscape phone preview, with additional compact and
large desktop coverage. Fixture-backed career/results are identified in the
capture output, and live navigation/chat/shop checks use isolated local servers.

The phone runs use the desktop development preview and are not physical-device,
native IME, StoreKit, signing, or network-release certification. No production
services, visibility rules, fog mechanics, game balance, or dependencies changed.
No release has been published.
