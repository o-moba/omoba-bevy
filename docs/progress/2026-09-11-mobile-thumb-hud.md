# Right-thumb HUD refinement — 11 September 2026

This refines the same uncommitted, unreleased `0.18.0-rc.7` mobile beta candidate.

## Layout

The user supplied a Wild Rift match screenshot showing the desired composition:
minimap upper left, joystick lower left, large attack at lower right, and skills
following an arc above and to its left. Riot's historical [aiming guide](https://wildrift.leagueoflegends.com/pl-pl/news/game-updates/popracuj-nad-celowaniem-i-namierzaniem-celow/)
was also inspected. Omoba retains its own abilities and artwork.

HP, mana, level and XP move to the upper-right area beside the menu, as requested.
The shop moves below the minimap. The objective panel occupies the space between
the map and the HP card and uses shorter phone hints. The combat fan uses the
same geometry for rendering and touch ownership, including separate rank-up
targets. Desktop layout keeps its existing behavior.

At 844x390, ATTACK remains at (764,322), with W/E/R at (673,285),
(704,224) and (776,224). Their diameters remain 88/62/62/68 logical pixels;
upgrade targets are 44 pixels. Main skill spacing stays above 6 pixels at the
compact 667x375 viewport. The fan reaches 22 fewer pixels upward than the
previous layout. These dimensions scale with the existing phone safe gutters.

The user then suggested fixing HP/mana above the hero's head, and explicitly
authorized keeping the current static interface for now. This iteration keeps
the static card. Larger, more readable local overhead bars remain a follow-up;
the existing world combat bars are unchanged.

## Verification

The frozen spec and current-run artifacts live in
`.agent/tasks/MOBILE-THUMB-HUD-2026-09-11/`. Native captures cover compact and wide
phone dimensions plus desktop. An extra opt-in, visibly labelled progression
fixture exposes all upgrade controls to inspect crowding. It changes client QA
presentation only and is not a server progression or full-match test.

Current verification passed 213 client tests, native build, formatting and
workspace Clippy with warnings denied. Six actual Bevy capture runs passed all
42 stages: normal HUD at 667x375, 844x390, 932x430 and desktop 1280x720; explicit
upgrade fixtures at 667x375 and 844x390. Gameplay/upgrade images were inspected;
all four upgrade targets are visible and the objective now participates in
overlap checks. Compact shop cards use smaller type and spacing so wrapped
descriptions and affordability text remain inside their borders; the avatar
hint fits beside the menu. Receipt-confirmed purchases still pass.

Physical-phone reach, cutouts, thermals and lifecycle still require device tests.
The [mobile beta record](2026-09-11-mobile-beta.md) retains packaging, SDK license
and public-server release gates. This HUD iteration does not deploy a beta.
