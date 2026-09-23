# Forest and combat VFX — 2026-09-23

Task: `TASK-FOREST-COMBAT-VFX-2026-09-23`. Version: `0.23.0-rc.3`.

The old butterfly presentation was tiny and placed in decorative jungle blocks, while impact sparks were too small and brief. Moved collectible flocks to shared walkable clearings, increased their height/contrast, added glow billboards and preserved flapping/drifting motion. Added server collection rules, replicated availability and monotonic receipts. Enhanced projectile silhouettes, orbiting magic sparks, trails and confirmed impact bursts. Added a transparent-center edge mist below the HUD without changing team vision.

No production dependencies or deployment changes. Projectile balance is unchanged; healing pickups are the only new gameplay mechanic. Physical iPad validation remains a separate device check; task proof records actual desktop native 3D/Sprite2d captures, including mobile-sized controls, and server/client regressions.

Proof and raw verification: `.agent/tasks/TASK-FOREST-COMBAT-VFX-2026-09-23/`. Implementation details and reproduction: `docs/forest-combat-vfx.md`.

User direction at final review: prioritize the 3D game. Sprite2d is paused; do not continue its visual development or QA unless explicitly requested. Completed compatibility checks are historical evidence, not an ongoing workstream.
