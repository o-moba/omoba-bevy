# 2D release vertical slice — 2026-07-27

## Outcome

The selectable sprite renderer was expanded from an idle/run player prototype
into a coherent combat presentation. Five heroes now have separate transparent
8×4 action sheets for attack, cast, hit, and death. The server publishes a
default-safe cosmetic action sequence/kind/slot only after accepting a cast;
local and remote clients therefore play the same one-shot exactly once. HP
deltas drive hit, authoritative zero HP holds death, and respawn returns to
locomotion under the priority `death > hit > attack/cast > run > idle`.

Sprite mode also replaces required replicated primitives/GLBs with cached
camera-facing presentation art for structures, minions, neutrals, both bosses,
and projectiles. Short-lived cast, hit, heal, and death effects reuse a single
atlas and self-clean. The existing arena geometry/collision stays unchanged but
uses a painted terrain treatment and hides the primitive decor layer in 2D.
The default `models3d` route is preserved.

## Art pipeline

The task's Art Director audit and style bible is stored at
`.agent/tasks/TASK-2D-RELEASE-VERTICAL-SLICE/art-direction.md`. Original
AI-assisted bitmap sources were produced with OpenAI image generation, cleaned
to straight-alpha PNGs, assembled into fixed grids, and validated locally.
Prompts, source outputs, contact boards, command logs, and QA notes stay below
the task's `raw/` directory; shipped provenance is recorded beside the assets.

Runtime metadata is split between:

- `client/assets/sprites/manifest.json` schema v2: five locomotion/action sets;
- `client/assets/presentation2d/manifest.json`: arena, actor, VFX, portrait, and
  UI asset paths plus atlas ranges, pivots, and world sizes.

`python3 scripts/validate_sprite_assets.py --self-test` validates safe paths,
dimensions, alpha, grids, nonempty/distinct animation frames, ranges, required
presentation categories, and the combined size budget.

## Verification scope

Headless transition tests cover action priority, one-shot completion, long
frame deltas, death hold, respawn, missing assets, and independent entities.
The real UDP harness covers two sequential accepted casts, rejected cooldown
casts, and backward/default-safe wire decoding. Full workspace format, build,
tests, strict clippy, asset validation, and diff checks are recorded in the
task evidence. Native GPU evidence remains an explicit environment-dependent
check and must not be inferred from headless results.
