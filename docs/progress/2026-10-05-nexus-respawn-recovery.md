# Nexus animation and respawn HUD recovery

> Consolidation follow-up: this historical worktree report is superseded for delivery state by [the main consolidation report](2026-10-05-main-consolidation.md). Its code is included in the combined 0.42.0 candidate; original verification/device limitations below remain historical evidence.

The earlier statement that all iteration work had reached main was incorrect. Main and origin/main were synchronized, but the separate playtest-round-two worktree retained the uncommitted 0.42 quality iteration. That worktree already contained the authored Nexus animation and enemy death portraits. The previous main fix added extra rings high above the Nexus instead of animating its existing central armillary geometry.

This focused 0.41.1 patch recovers the two requested features without pulling in the unrelated map, combat, dragon and practice changes:

- Split the original sanctuary armillary rings into named GLB nodes around the crystal at local Y=7.33; preserve all original triangles/materials and static structure. Remove additive upper rings. Animate node pivots only, excluding render primitives and unrelated scene instances.
- Add a backward-compatible public respawn duration to scoreboard rows, supplied by the same authority helper for online/offline snapshots. Grey enemy death portraits have a countdown and cannot acquire a target; living hidden enemies remain excluded. Allied portraits also show countdowns.
- Quantize the ally HUD cache key to displayed seconds to avoid rebuilding it for every millisecond change.
- Restore the already corrected source inventory hashes for the provenance document and saved Blender file; no source art was modified.

Verification and exact command results are stored in `.agent/tasks/NEXUS-RESPAWN-RECOVERY/`. Device installation, TestFlight upload and server deployment are separate from this source recovery. Online countdowns require the updated server; an older compatible server defaults the new field to zero.

## Work still outside main

The rest of the uncommitted playtest quality iteration remains preserved in `_workspace/worktrees/playtest-round-two` (0.42). It includes combat/practice changes, ranged weapons, jungle/dragon/map changes and other visual/HUD fixes. Trailer work is separately preserved in `_workspace/worktrees/trailer-v2`. Neither is claimed merged by this focused fix. Inventory is recorded in the task evidence, and these worktrees must not be discarded as empty or already merged.

## Verification result and release state

Current-source client/server test compilation, authority/offline timer tests, backward-compatible JSON tests, enemy portrait lifecycle tests, Nexus animation/asset tests and existing tactical HUD tests passed. Independent asset validation passed for all six environment GLBs. The new ally timer overlay was compiled and reviewed; no physical iOS validation or new screenshot capture was performed.

This patch currently lives in the uncommitted `fix/nexus-respawn-recovery` worktree, not main. Full release checks and mobile builds were not attempted after disk free space reached 17 GiB (below the documented 20 GiB threshold). No caches were deleted, server deployed or TestFlight build uploaded.
