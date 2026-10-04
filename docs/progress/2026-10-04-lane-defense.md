# Configurable lane defense and lane brush — 2026-10-04

## Gameplay and configuration

Version 0.39.0 gives each team three towers on each of the three lanes: outer, inner and Nexus-side. The map contains 18 lane towers and two existing Nexus/base structures. Original IDs 1–8, outer positions and combat statistics are retained. A tower unlocks only after all earlier active ranks on its lane fall; clearing a defended lane opens the enemy base under the existing rules. Additional defensive stops can extend match duration; no duration or win-rate claim is made without playtesting.

The optional `disabled_tower_tiers` array disables authored ranks before building simulation objects. Validation still checks the complete authored layout. Surviving ranks and IDs stay stable, including gaps. The shared resolver feeds collision, authoritative targeting, snapshots, the world renderer and minimap, so disabled towers leave no invisible obstacles or markers. Complete one- and two-tower presets are included in `examples/maps/`; [map customization](../map-customization.md) explains host startup, validation and reset behavior. Offline Practice uses the embedded three-tier default.

Six new symmetric lane-edge hiding pockets supplement the existing ten jungle pockets. They are reachable from lanes, preserve minion centerlines and clear structure sight footprints. The same shared geometry drives online/offline concealment and 3D/2D art. Entering a pocket hides the hero from enemies outside it; hostile actions reveal temporarily, and exiting restores ordinary visibility. Existing concealment HUD feedback remains in use.

Protocol 8 requires matching clients and hosts because brush geometry is compiled into both. Version changes, configuration and source are local to `feat/playtest-round-two`; this task does not deploy the server, push/merge code or publish a mobile build. Prior playtest, Adventurer and movement-reversal changes remain preserved.

## Verification

Current workspace tests pass: 1,515 passed, zero failed, 39 intentionally ignored. Coverage includes all eight disabled-tier subsets, malformed settings, stable identities and ranks, sequential damage/protection, collision, minion targeting, reset, and real offline conceal/reveal/exit. Independent verification checks the current source and binaries; full results and raw evidence live in `.agent/tasks/LANE-DEFENSE-20261004/`.

Two prior combat fixtures now explicitly select an outer tower, rather than a random potentially protected tower. The pickup fixture includes the extra sight supplied by allied inner towers. The wire-enum policy pin was reviewed and advanced with the protocol. Production protection and visibility rules were preserved.

Native screenshot verification is limited to one English desktop phone preview at 852 × 393, showing the real locally hosted layout. It is not physical iPhone performance or touch-play evidence. Historical ignored 2D image-generation provenance is absent in this worktree; current manifest, topology and PNG pixels are checked separately. No assets or dependencies were added for tower or bush art.

The real-server integration checks passed (7 tests): full 20-object map, custom map pinning, one-/two-tier variants, full 5v5 framed snapshots, malformed-packet recovery and legacy datagram compatibility. Python capture-verifier tests passed (36), as did 2D contract negative tests (9), current manifest/PNG validation, workspace clippy with warnings denied, formatting and whitespace checks.

Native final capture passed for both planned frames: map overview and Green lane defense/brush detail. Readback checks matched 20 structures to original server receipts, all 16 shared brush pockets to rendered geometry, and all six primary touch controls. The first attempts exposed stale QA menu selection and desktop window focus; both were fixed in the QA harness. The final macOS run also required explicitly foregrounding its unbundled process. Screenshots are layout evidence only; their FPS label includes a prior background interval. No additional viewport or language matrix was run.

Fresh independent verification passed AC1–AC5 with 55 focused checks, read-only re-verification of both final captures, and no unresolved findings.
