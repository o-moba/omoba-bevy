# Playtest quality iteration — 2026-10-05

> Consolidation follow-up: this historical worktree report is superseded for delivery state by [the main consolidation report](2026-10-05-main-consolidation.md). Its code is included in the combined 0.42.0 candidate; original verification/device limitations below remain historical evidence.

Status: **implemented and locally verified on `feat/playtest-quality-three`**.
Workspace version 0.42.0; production Beta and uploaded iOS remain 0.41.0 (20).
No deployment, mobile build, commit or push is implied by this note.

## Priority and acceptance map

| Priority | User items | Result | Evidence / limit |
| --- | --- | --- | --- |
| P0 | 2,4 | TH basic attacks resolve at melee contact, damaging clips thrust with the dagger hand, close/manual casts include the swept reach, rejected predicted cooldowns release after the grace period. Rank 1 baseline: 24 AA / 0.7s; rear combo166 before growth/items. | Shared combat and targeting/cooldown regressions; real-device feel still needs playtesting. |
| P0 | 18 | Successful chat send closes the modal, IME state changes only on transitions, native editor pacing keeps processing events. | Confirmed input-latch fix; physical iOS freeze has not been reproduced here. |
| P0 | 11 | Victory survives late conflicting results and empty-roster teardown; Details and locale rebuild retain the same accepted receipt. | Client/session lifecycle regressions. |
| P1 | 1,7 | Practice-only zero cooldowns beside god mode; scrollable tools; stationary, moving passive and aggressive disposable targets. Targets disappear on death. | Same shared handler online/offline, permission/lifetime/scroll regressions. |
| P1 | 6,10 | Local terrain-anchored ring replaces redundant triangles. Accepted enemy attack sequences trigger bounded audio even if a shield absorbs damage. | Grounding and audio receipt/dedup/fog tests; physical phone audio not checked. |
| P1 | 12,17 | Default hand-attached repeater/launcher, bow and scepter; explicit SDK equip/unequip wins. Repeater fires single-target at 0.7s before stacks; rockets at 1.19s cost 4 mana and splash. | Shared attack/manifest/cosmetic parsing tests; bow draw and two-hand IK deferred. |
| P1 | 13 | Public enemy death portraits count down in compact right-side slots. Dead portraits cannot target or expose hidden positions. | Authoritative countdown and stale-target tests; native phone viewport capture below. |
| P2 | 3 | Ten ordinary camps; orange fire/blue ice minimap cues. Fire gives +4 basic damage; ice gives +2 and 10% slow for 1s. Rewards last 45s, refresh without stacking and do not affect structures. | Common offline/hosted hit pipeline and expiry/reset tests; personal buff HUD timers deferred. |
| P2 | 5,14 | Original rigged Verdant Dragon replaces King Mutatio's 3D presentation. Three nexus rings rotate separately around the crystal. | Original Blender sources, asset validation, topology/pivot and animation ownership tests. Optional 2D boss sprite is unchanged. |
| P2 | 8,9 | Shared map XZ scale 0.8, river 12m, matched terrain/collision/brush/minimap. Brush uses an opaque depth-writing footprint; thin moving blades do not cast/receive unstable shadows. | Geometry/visibility tests and native capture below; geometry scale is compiled, not a runtime map editor. |
| P2 | 15,16 | Base-side towers use a shorter silhouette; outer/middle tiers advance. Default tower/base ranges 16/19.2m preserve proportionate coverage. | Custom explicit ranges and tier protection remain intact; map/AI regression suite. |

The compact map exposed two existing AI path problems: approach goals could
land inside a tree, and a live tower could occupy a lane-corner waypoint.
Bots now route toward the real target, stop at reach and advance reached
projected lane endpoints while still respecting unsupported-tower staging.

## Architecture and delivery

Combat, jungle rewards and practice permissions remain in `common`, shared by
server and offline simulation. Enemy respawn deadlines are projected to a
public countdown; no private location is added. Assets have editable Blender
sources, provenance and package gates. The map has one compiled scale consumed
by authority and renderers; custom per-match scale remains future work.

Compatibility contract: protocol 10, `standard-kits-5`,
`verdant-confluence-compact-v2`, `combat-2026-10-05`. Matching client and server
must be released together; old peers are rejected by the preflight handshake.

Files span shared/common combat and map data, server practice/session snapshots,
client HUD/input/3D presentation, original weapon/dragon assets and tooling.
The Chinese dragon glyph subset is synchronized from `omoba-ui/handoff/assets`;
that repository has two corresponding asset changes to include when publishing.

## Verification

- Full `make check`: 1,588 Rust tests passed (39 deliberately ignored database/device cases), strict Clippy with/without QA passed; 164 script tests and 45 iOS tooling tests passed. iOS tooling tests use fixtures; no phone installation/build is claimed.
- Live UDP harness against the freshly built 0.42.0 server: 50 tests passed, including full matchmaking, combat, boss timing, jungle farming/respawn and session behavior.
- Fresh review repaired two alternate stale-result paths (Details and locale rebuild). Native debug capture then exposed overlapping career shortcuts; the final submenu visibility regression and client Clippy pass independently after that fix.
- Candidate asset policy/legal/font checks pass. Independent Verdant validation retains all sanctuary triangles/materials/UVs/normals and verifies three ring pivots plus deterministic derivation. Two stale source pins were reconciled to already-committed baseline files after byte-level review; no unreviewed art changes were accepted.
- Native EN 852×393 captures pass and were visually inspected: compact enemy death countdown, readable local chat after successful submission, and scrolled debug tools with an unobstructed Back button. HUD is explicitly a synthetic presentation fixture; offline chat/tools use production handlers, with simulated window focus and a fixture scroll offset. Actual touch scrolling has a separate layout/input regression.
- Two native arena frames pass against the local server: the real living dragon appears on its normal 180-second schedule with matching independently observed identity/HP; the hero then returns to the base for the nexus/inner-tower view. Native OS focus, gameplay input and primary control bounds were valid. No spawn cheats, teleport or synthetic world actors were used (the independent observer is a second admitted player). Ring motion also has a deterministic ownership/pivot regression.
- Final client binary SHA-256: `101acbbf89997cde3c718d9ae6a29f346a1ff366b74c28f90ee5a13891efe49d`; server: `8b4e672f203819f01443c7583cff606c72251e13aee950bed09340a7e3b30c0d`. The earlier HUD/chat captures use client `f8ac9bebf247f70e14480ea4c7f5369ad8568854a85b95dc64bcd0313ceac25f`, before the isolated career-submenu visibility repair; only its affected debug state was recaptured.

Raw logs, original failures, focused reruns and fresh-review findings live in
`.agent/tasks/PLAYTEST-QUALITY-20261005/`. A concurrent recording task replaced
the shared client executable during the first capture attempt; the handshake
rejected 0.41/0.42 as expected. Subsequent checks use hashed copies of this
iteration's client/server. The first map capture lacked native OS focus and
produced no images; its retry activates only the specific QA process.

Routine visual scope: English, 852×393 desktop native renderer with phone
controls, affected states only. This is not physical iOS/Android certification,
performance measurement, a full match balance study or remote deployment.

Follow-ups: [refactoring and device TODO](../REFACTORING.md#quality-iteration-follow-ups--2026-10-05).

## Retained artifacts and remaining limits

The source remains uncommitted in this worktree. No production infrastructure,
secrets, CI configuration or dependencies changed. New asset files must be added
to Git before native release packaging (the packager uses tracked files).
The matching font/manifest edits in `omoba-ui` also remain uncommitted.

Fixed executable copies and five passing captures are retained under
`.agent/tasks/PLAYTEST-QUALITY-20261005/raw/` (approximately 433 MB of binaries,
plus logs/snapshots). No third Cargo cache was created or existing cache removed.
Free disk space fell below the repository's 20 GB threshold after builds;
further heavy builds were stopped. All task-owned test clients/servers exited.

Remaining limits: physical iPhone UIKit/audio/120 Hz are unverified; live match
balance needs real playtests. Bow two-hand IK, individual jungle-buff timers and
the optional 2D dragon sprite are explicit follow-ups, not claimed complete.
