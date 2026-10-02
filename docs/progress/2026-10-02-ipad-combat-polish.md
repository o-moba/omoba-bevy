# iPad combat polish — 2026-10-02

Branch: `fix/ipad-combat-polish`, base `5aa2fe4`, patch version 0.34.1.

The visible mobile + badges were decorative children outside the ability hit circles. They now share an explicit screen anchor with a direct upgrade gesture and a 44-point target. The world navigation ownership check recognizes this region. Rank mode remains available. Cancellation, another finger, and a server-side eligibility change do not emit a cast or upgrade.

Basic attack misses are silent and attack aim no longer displays an instructional text box. Skill feedback and the visual targeting/cancel controls remain. Kill rows use the safe right edge below the top HUD.

Concealment follows `TeamVision.local_hidden` during a running 3D match. Local mesh bindings temporarily use 45% opacity copies, including late-loaded handheld meshes. Reveal, absent vision, and changed bindings restore the correct original handles without modifying shared source assets.

## Verification

Task evidence: `.agent/tasks/IPAD-COMBAT-POLISH-20261002/`.
Selected visual scope: English, iPad landscape 1180×820 logical points, affected states only. Viewport evidence does not imply physical iPad verification.

Owner authorized stale cache cleanup: removed 27 obsolete client rlibs (16.93 GiB) and 43 old client/test executables (6.37 GiB), plus 284 obsolete workspace test executables (9.29 GiB), preserving newest outputs, final packages and signing material. Total reclaimed: 32.59 GiB.

837 client tests and 90 common tests passed, including real Bevy/Taffy overhead layout and rocket sight/marker lifecycle regressions. Initial sandbox-only network failures were resolved by running loopback fixtures outside the sandbox. The full gate found an existing clippy function-order error in `frontend/collection.rs`; the two handheld helpers moved before its tests without behavior changes. Workspace formatting, both clippy modes and all 1,385 Rust tests pass. The helper test IDs were explicitly marked non-display identifiers for the localization scanner. Registered the focused QA fixture in the screen map; all 155 script tests and 44 iOS tooling tests pass.

English native captures in `raw/native-v3/` show fixed vitals, direct-upgrade controls, silent held attack, right-side kill feed, concealment and the rocket model/light. These are explicitly labeled client presentation fixtures, not real multiplayer or device proof. The minimap marker renders above hero portraits; the final ground-glow plane adjustment avoids overlap with the existing telegraph.

Actual device readback: installed 0.34.0 (14). No replacement iPad installation or production update has yet occurred for this patch.

## Additional fixes

Overhead nickname, HP and mana have independent fixed rows, preventing flex shrink from making bars disappear. Existing 3D bars remain replaced by mobile screen-space vitals.

The server now sends tower landmarks to admitted players outside sight, without changing `target_visible` combat checks. Impact rockets provide an eight-metre vision source to their team at their current simulated position; the source disappears with the effect. Clients draw only server-filtered rocket positions on the minimap, plus a bounded light and brighter exhaust. No wire schema, database migration or infrastructure change is needed; the existing server binary must be updated for tower replication and rocket vision.

Linux release binary builds successfully with Rust 1.94.1 in the retained cache. Physical iPad package 0.34.1 (16) is being prepared; production remains on 0.34.0 pending the separately reviewed binary rollout.
