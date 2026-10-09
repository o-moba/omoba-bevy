# Arena surroundings, Dash and offline Play Again — 2026-10-06

Candidate: 0.44.0, `fix/mobile-playtest-quality`. No server deployment, mobile
archive, commit, push or merge is part of this follow-up.

## Changes

- `client/src/verdant3d/surroundings.rs` adds one persistent, unlit, non-colliding
  world-space matte below the island's existing cliff geometry. The original
  Higgsfield forest image is packaged as `client/assets/verdant/surroundings.png`;
  `SURROUNDINGS.md` records its model, job, prompt and SHA-256. Mirrored UV wrapping
  keeps detail at forest scale without enlarging the playable area or downloading
  content at runtime. One additional plane/material, one 2048-square texture.
- `shared/src/navigation` finds the furthest safe landing along a bounded blink
  ray, without testing intermediate collision. `common/src/utility.rs` supplies
  both living structure footprints and active temporary terrain. The client
  Dash preview uses the same landing algorithm and its visible obstacle state.
  Hero clearance, bounds, control gates, cooldown and sequence protection remain.
- `client/src/combat/round_reset.rs` discards the already delivered RequestRematch
  when clearing old-round input. Previously it drained retained Bevy messages
  and wrote this command back with a new message ID; each new round triggered
  another replay. `common/src/offline.rs` additionally admits rematch only from
  Victory, as the online server already does.
- Gameplay compatibility revision is `combat-2026-10-06-blink`. The new online
  Dash behavior needs the matching server build; this document does not claim
  that Beta has been deployed.

## Evidence

Native English mobile UI at 1180×820, real offline host and production result
button/input/network pipeline. QA injects an authoritative Victory to end each
round and a movement destination to exercise movement; this is not a physical
finger/device test or a fully played match. The server endpoint is deliberately
unreachable; all gameplay runs in-process.

Before the fix: one Play Again advances round 1 to 113 in about three seconds;
local displacement is 0 m and bots only 0.0038 m. UI remains responsive.
After both fixes: two consecutive Victory → Play Again cycles advance exactly
1 → 2 → 3. Local displacement is 5 m in both; bots move approximately 10 m.
The input gate remains open. Source-specific tracing found the first request
from the result button and its duplicate on RoundChanged; temporary tracing was
removed after attribution.

Validation: 33 focused Rust tests passed (10 navigation, 2 compatibility, 11 offline
host, 9 server utility, 1 client round-reset). Final native build, strict Clippy
with and without QA, formatting and whitespace checks passed.

Focused coverage: static and dynamic landing clearance, map bounds, cooldown,
identity/replay admission, old transform sequence rejection; terminal-only and
repeated offline rematch; client message retention/replay regression. Candidate
asset/provenance gate also passes. Raw logs, JSON and screenshots live under
`.agent/tasks/WORLD-DASH-REMATCH-20261006/` (local, deliberately untracked).

## Limits

iPhone/iPad hardware has not been retested here. This is a low-cost distant matte,
not new traversable 3D terrain. Camera coverage is checked at opposite arena
corners, including maximum gameplay zoom; unrestricted debug free flight can
leave the bounded backdrop. Online preview only knows visible replicated
obstacles; the server owns the final landing.
